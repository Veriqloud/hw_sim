use std::{
    error::Error,
    fmt,
    io::{self, BufRead, BufReader, Write},
    os::unix::net::UnixStream,
    path::{Path, PathBuf},
    thread,
    time::Duration,
};

use simulator::runtime_control::{CommandRequest, CommandResponse};

const COMMAND_TIMEOUT: Duration = Duration::from_secs(5);
const RECALIBRATION_TIMEOUT_SLACK: Duration = Duration::from_secs(30);

#[derive(Debug)]
pub enum ControlError {
    Io {
        operation: &'static str,
        path: PathBuf,
        source: io::Error,
    },
    InvalidResponse {
        path: PathBuf,
        source: serde_json::Error,
    },
    EmptyResponse {
        path: PathBuf,
    },
    Rejected {
        path: PathBuf,
        message: String,
    },
}

impl fmt::Display for ControlError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io {
                operation,
                path,
                source,
            } => write!(
                formatter,
                "failed to {operation} on {}: {source}",
                path.display()
            ),
            Self::InvalidResponse { path, source } => write!(
                formatter,
                "invalid response from {}: {source}",
                path.display()
            ),
            Self::EmptyResponse { path } => {
                write!(formatter, "empty response from {}", path.display())
            }
            Self::Rejected { path, message } => {
                write!(
                    formatter,
                    "{} rejected the command: {message}",
                    path.display()
                )
            }
        }
    }
}

impl Error for ControlError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::InvalidResponse { source, .. } => Some(source),
            Self::EmptyResponse { .. } | Self::Rejected { .. } => None,
        }
    }
}

#[derive(Debug)]
pub struct PairCommandError {
    failures: Vec<String>,
}

impl fmt::Display for PairCommandError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.failures.join("; "))
    }
}

impl Error for PairCommandError {}

pub fn send_command_to_pair(
    alice_socket_path: impl AsRef<Path>,
    bob_socket_path: impl AsRef<Path>,
    command: &CommandRequest,
) -> Result<(), PairCommandError> {
    send_requests_to_pair(
        alice_socket_path,
        bob_socket_path,
        command,
        command,
        COMMAND_TIMEOUT,
    )
    .map(|_| ())
}

pub fn recalibrate_pair(
    alice_socket_path: impl AsRef<Path>,
    bob_socket_path: impl AsRef<Path>,
    duration_ms: u64,
) -> Result<(), PairCommandError> {
    let alice_socket_path = alice_socket_path.as_ref();
    let bob_socket_path = bob_socket_path.as_ref();
    let pause = CommandRequest::Pause { duration_ms };
    let (alice_paused, bob_paused) = send_requests_to_pair(
        alice_socket_path,
        bob_socket_path,
        &pause,
        &pause,
        RECALIBRATION_TIMEOUT_SLACK,
    )?;

    let alice_progress = response_progress("Alice", alice_paused)?;
    let bob_progress = response_progress("Bob", bob_paused)?;
    if alice_progress.batch_pulse_count == 0
        || alice_progress.batch_pulse_count != bob_progress.batch_pulse_count
    {
        return Err(protocol_error(format!(
            "incompatible batch pulse counts: Alice={}, Bob={}",
            alice_progress.batch_pulse_count, bob_progress.batch_pulse_count
        )));
    }

    let event_difference = alice_progress
        .event_count
        .abs_diff(bob_progress.event_count);
    if event_difference % alice_progress.batch_pulse_count != 0 {
        return Err(protocol_error(format!(
            "generation progress differs by {event_difference} events, which is not a whole number of batches"
        )));
    }
    let missing_batches = event_difference / alice_progress.batch_pulse_count;
    let (alice_discard, bob_discard) = if alice_progress.event_count < bob_progress.event_count {
        (missing_batches, 0)
    } else {
        (0, missing_batches)
    };

    send_requests_to_pair(
        alice_socket_path,
        bob_socket_path,
        &CommandRequest::Synchronize {
            batches_to_discard: alice_discard,
        },
        &CommandRequest::Synchronize {
            batches_to_discard: bob_discard,
        },
        COMMAND_TIMEOUT,
    )?;

    let completion_timeout =
        Duration::from_millis(duration_ms).saturating_add(RECALIBRATION_TIMEOUT_SLACK);
    send_requests_to_pair(
        alice_socket_path,
        bob_socket_path,
        &CommandRequest::Resume,
        &CommandRequest::Resume,
        completion_timeout,
    )?;
    Ok(())
}

fn send_requests_to_pair(
    alice_socket_path: impl AsRef<Path>,
    bob_socket_path: impl AsRef<Path>,
    alice_command: &CommandRequest,
    bob_command: &CommandRequest,
    timeout: Duration,
) -> Result<(CommandResponse, CommandResponse), PairCommandError> {
    let alice_socket_path = alice_socket_path.as_ref();
    let bob_socket_path = bob_socket_path.as_ref();

    let (alice_result, bob_result) = thread::scope(|scope| {
        let alice = scope.spawn(|| send_command(alice_socket_path, alice_command, timeout));
        let bob = scope.spawn(|| send_command(bob_socket_path, bob_command, timeout));
        (alice.join(), bob.join())
    });

    let mut failures = Vec::new();
    let alice_response = collect_result("Alice", alice_result, &mut failures);
    let bob_response = collect_result("Bob", bob_result, &mut failures);

    if failures.is_empty() {
        Ok((alice_response.unwrap(), bob_response.unwrap()))
    } else {
        Err(PairCommandError { failures })
    }
}

fn send_command(
    socket_path: &Path,
    command: &CommandRequest,
    timeout: Duration,
) -> Result<CommandResponse, ControlError> {
    let mut stream = UnixStream::connect(socket_path)
        .map_err(|source| io_error("connect", socket_path, source))?;
    stream
        .set_read_timeout(Some(timeout))
        .map_err(|source| io_error("set read timeout", socket_path, source))?;
    stream
        .set_write_timeout(Some(timeout))
        .map_err(|source| io_error("set write timeout", socket_path, source))?;

    serde_json::to_writer(&mut stream, command)
        .map_err(|source| io_error("write command", socket_path, io::Error::other(source)))?;
    stream
        .write_all(b"\n")
        .and_then(|_| stream.flush())
        .map_err(|source| io_error("write command", socket_path, source))?;

    let mut response_line = String::new();
    BufReader::new(stream)
        .read_line(&mut response_line)
        .map_err(|source| io_error("read response", socket_path, source))?;
    if response_line.is_empty() {
        return Err(ControlError::EmptyResponse {
            path: socket_path.to_owned(),
        });
    }

    let response: CommandResponse =
        serde_json::from_str(&response_line).map_err(|source| ControlError::InvalidResponse {
            path: socket_path.to_owned(),
            source,
        })?;

    if response.status == "ok" {
        Ok(response)
    } else {
        Err(ControlError::Rejected {
            path: socket_path.to_owned(),
            message: response
                .message
                .unwrap_or_else(|| format!("status {}", response.status)),
        })
    }
}

fn collect_result(
    simulator: &str,
    result: thread::Result<Result<CommandResponse, ControlError>>,
    failures: &mut Vec<String>,
) -> Option<CommandResponse> {
    match result {
        Ok(Ok(response)) => Some(response),
        Ok(Err(error)) => {
            failures.push(format!("{simulator}: {error}"));
            None
        }
        Err(_) => {
            failures.push(format!("{simulator}: command worker panicked"));
            None
        }
    }
}

fn response_progress(
    simulator: &str,
    response: CommandResponse,
) -> Result<simulator::runtime_control::GenerationProgress, PairCommandError> {
    response
        .progress
        .ok_or_else(|| protocol_error(format!("{simulator}: pause response has no progress")))
}

fn protocol_error(message: String) -> PairCommandError {
    PairCommandError {
        failures: vec![message],
    }
}

fn io_error(operation: &'static str, path: &Path, source: io::Error) -> ControlError {
    ControlError::Io {
        operation,
        path: path.to_owned(),
        source,
    }
}

#[cfg(test)]
mod tests {
    use super::{recalibrate_pair, send_command_to_pair};
    use simulator::runtime_control::{CommandRequest, CommandResponse, GenerationProgress};
    use std::{
        io::{BufRead, BufReader, Write},
        os::unix::net::UnixListener,
        path::PathBuf,
        thread,
    };
    use uuid::Uuid;

    #[test]
    fn sends_the_same_command_to_both_simulators() {
        let alice_path = socket_path("alice");
        let bob_path = socket_path("bob");
        let alice = spawn_server(alice_path.clone(), ok_response());
        let bob = spawn_server(bob_path.clone(), ok_response());
        let command = CommandRequest::StartAttack;

        send_command_to_pair(&alice_path, &bob_path, &command).unwrap();

        assert_eq!(alice.join().unwrap(), command);
        assert_eq!(bob.join().unwrap(), command);
    }

    #[test]
    fn reports_a_rejection_after_contacting_both_simulators() {
        let alice_path = socket_path("alice");
        let bob_path = socket_path("bob");
        let alice = spawn_server(
            alice_path.clone(),
            CommandResponse {
                status: "error".to_owned(),
                message: Some("pause already pending or running".to_owned()),
                progress: None,
            },
        );
        let bob = spawn_server(bob_path.clone(), ok_response());
        let command = CommandRequest::StartAttack;

        let error = send_command_to_pair(&alice_path, &bob_path, &command).unwrap_err();

        assert!(error.to_string().contains("Alice"));
        assert!(error
            .to_string()
            .contains("pause already pending or running"));
        assert_eq!(alice.join().unwrap(), command);
        assert_eq!(bob.join().unwrap(), command);
    }

    #[test]
    fn recalibration_discards_only_the_lagging_simulator_batches() {
        let alice_path = socket_path("alice-recalibration");
        let bob_path = socket_path("bob-recalibration");
        let alice = spawn_recalibration_server(
            alice_path.clone(),
            GenerationProgress {
                event_count: 20,
                batch_pulse_count: 10,
            },
        );
        let bob = spawn_recalibration_server(
            bob_path.clone(),
            GenerationProgress {
                event_count: 50,
                batch_pulse_count: 10,
            },
        );

        recalibrate_pair(&alice_path, &bob_path, 200).unwrap();

        let alice_requests = alice.join().unwrap();
        let bob_requests = bob.join().unwrap();
        assert_eq!(
            alice_requests[1],
            CommandRequest::Synchronize {
                batches_to_discard: 3,
            }
        );
        assert_eq!(
            bob_requests[1],
            CommandRequest::Synchronize {
                batches_to_discard: 0,
            }
        );
        assert_eq!(alice_requests[2], CommandRequest::Resume);
        assert_eq!(bob_requests[2], CommandRequest::Resume);
    }

    fn spawn_server(
        socket_path: PathBuf,
        response: CommandResponse,
    ) -> thread::JoinHandle<CommandRequest> {
        let listener = UnixListener::bind(&socket_path).unwrap();
        thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request_line = String::new();
            BufReader::new(stream.try_clone().unwrap())
                .read_line(&mut request_line)
                .unwrap();
            let request = serde_json::from_str(&request_line).unwrap();

            serde_json::to_writer(&mut stream, &response).unwrap();
            stream.write_all(b"\n").unwrap();
            stream.flush().unwrap();
            drop(listener);
            std::fs::remove_file(socket_path).unwrap();

            request
        })
    }

    fn socket_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("hw-sim-control-{name}-{}.socket", Uuid::new_v4()))
    }

    fn ok_response() -> CommandResponse {
        CommandResponse {
            status: "ok".to_owned(),
            message: None,
            progress: None,
        }
    }

    fn spawn_recalibration_server(
        socket_path: PathBuf,
        progress: GenerationProgress,
    ) -> thread::JoinHandle<Vec<CommandRequest>> {
        let listener = UnixListener::bind(&socket_path).unwrap();
        thread::spawn(move || {
            let mut requests = Vec::new();
            for response in [
                CommandResponse {
                    status: "ok".to_owned(),
                    message: None,
                    progress: Some(progress),
                },
                ok_response(),
                ok_response(),
            ] {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request_line = String::new();
                BufReader::new(stream.try_clone().unwrap())
                    .read_line(&mut request_line)
                    .unwrap();
                requests.push(serde_json::from_str(&request_line).unwrap());
                serde_json::to_writer(&mut stream, &response).unwrap();
                stream.write_all(b"\n").unwrap();
                stream.flush().unwrap();
            }
            drop(listener);
            std::fs::remove_file(socket_path).unwrap();
            requests
        })
    }
}
