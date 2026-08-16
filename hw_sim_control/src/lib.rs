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
    let alice_socket_path = alice_socket_path.as_ref();
    let bob_socket_path = bob_socket_path.as_ref();

    let (alice_result, bob_result) = thread::scope(|scope| {
        let alice = scope.spawn(|| send_command(alice_socket_path, command));
        let bob = scope.spawn(|| send_command(bob_socket_path, command));
        (alice.join(), bob.join())
    });

    let mut failures = Vec::new();
    collect_result("Alice", alice_result, &mut failures);
    collect_result("Bob", bob_result, &mut failures);

    if failures.is_empty() {
        Ok(())
    } else {
        Err(PairCommandError { failures })
    }
}

fn send_command(socket_path: &Path, command: &CommandRequest) -> Result<(), ControlError> {
    let mut stream = UnixStream::connect(socket_path)
        .map_err(|source| io_error("connect", socket_path, source))?;
    stream
        .set_read_timeout(Some(COMMAND_TIMEOUT))
        .map_err(|source| io_error("set read timeout", socket_path, source))?;
    stream
        .set_write_timeout(Some(COMMAND_TIMEOUT))
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
        Ok(())
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
    result: thread::Result<Result<(), ControlError>>,
    failures: &mut Vec<String>,
) {
    match result {
        Ok(Ok(())) => {}
        Ok(Err(error)) => failures.push(format!("{simulator}: {error}")),
        Err(_) => failures.push(format!("{simulator}: command worker panicked")),
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
    use super::send_command_to_pair;
    use simulator::runtime_control::{CommandRequest, CommandResponse};
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
        let command = CommandRequest::Pause { duration_ms: 200 };

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
        }
    }
}
