use std::{
    fs,
    io::{BufRead, BufReader, Write},
    os::unix::net::{UnixListener, UnixStream},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, Sender},
        Arc, Mutex,
    },
    thread,
    time::Duration,
};

use serde::{Deserialize, Serialize};

use crate::backend::actor::ActorHandle as SimulatorHandle;

#[derive(Clone)]
pub struct RuntimeControl {
    receiver: Arc<Mutex<Receiver<RuntimeCommand>>>,
    pause_in_progress: Arc<AtomicBool>,
}

impl RuntimeControl {
    pub fn try_recv(&self) -> Option<RuntimeCommand> {
        match self.receiver.lock() {
            Ok(receiver) => receiver.try_recv().ok(),
            Err(e) => {
                tracing::error!("Runtime control receiver lock poisoned: {}", e);
                None
            }
        }
    }

    pub fn complete_pause(&self) {
        self.pause_in_progress.store(false, Ordering::SeqCst);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeCommand {
    Pause { duration: Duration },
}

#[derive(Clone)]
struct RuntimeControlServer {
    sender: Sender<RuntimeCommand>,
    simulator_handle: SimulatorHandle,
    pause_in_progress: Arc<AtomicBool>,
}

#[derive(Debug, Deserialize)]
struct CommandRequest {
    command: String,
    duration_ms: Option<u64>,
}

#[derive(Debug, Serialize)]
struct CommandResponse {
    status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
}

#[derive(Debug)]
enum ParsedCommand {
    StartAttack,
    StopAttack,
    Pause { duration: Duration },
}

pub fn start_runtime_control_server(
    socket_path: impl Into<PathBuf>,
    simulator_handle: SimulatorHandle,
) -> Result<RuntimeControl, std::io::Error> {
    let socket_path = socket_path.into();
    remove_socket_if_exists(&socket_path)?;
    if let Some(parent) = socket_path.parent() {
        fs::create_dir_all(parent)?;
    }

    let listener = UnixListener::bind(&socket_path)?;
    let (sender, receiver) = mpsc::channel();
    let pause_in_progress = Arc::new(AtomicBool::new(false));
    let server = RuntimeControlServer {
        sender,
        simulator_handle,
        pause_in_progress: pause_in_progress.clone(),
    };

    thread::spawn(move || {
        tracing::info!(
            "Runtime control server listening on {}",
            socket_path.display()
        );
        for stream in listener.incoming() {
            match stream {
                Ok(stream) => {
                    let server = server.clone();
                    thread::spawn(move || handle_client(stream, server));
                }
                Err(e) => tracing::error!("Runtime control socket accept failed: {}", e),
            }
        }
    });

    Ok(RuntimeControl {
        receiver: Arc::new(Mutex::new(receiver)),
        pause_in_progress,
    })
}

fn handle_client(stream: UnixStream, server: RuntimeControlServer) {
    let reader_stream = match stream.try_clone() {
        Ok(stream) => stream,
        Err(e) => {
            tracing::error!("Could not clone runtime control stream: {}", e);
            return;
        }
    };
    let reader = BufReader::new(reader_stream);
    let mut writer = stream;

    for line in reader.lines() {
        let response = match line {
            Ok(line) => server.handle_line(&line),
            Err(e) => CommandResponse::error(format!("read failed: {}", e)),
        };

        match serde_json::to_vec(&response) {
            Ok(mut payload) => {
                payload.push(b'\n');
                if let Err(e) = writer.write_all(&payload).and_then(|_| writer.flush()) {
                    tracing::error!("Could not write runtime control response: {}", e);
                    return;
                }
            }
            Err(e) => {
                tracing::error!("Could not serialize runtime control response: {}", e);
                return;
            }
        }
    }
}

impl RuntimeControlServer {
    fn handle_line(&self, line: &str) -> CommandResponse {
        match parse_command(line) {
            Ok(ParsedCommand::StartAttack) => match self.simulator_handle.start_attack() {
                Ok(()) => CommandResponse::ok(),
                Err(e) => CommandResponse::error(format!("start_attack failed: {}", e)),
            },
            Ok(ParsedCommand::StopAttack) => match self.simulator_handle.stop_attack() {
                Ok(()) => CommandResponse::ok(),
                Err(e) => CommandResponse::error(format!("stop_attack failed: {}", e)),
            },
            Ok(ParsedCommand::Pause { duration }) => {
                match enqueue_pause(&self.sender, &self.pause_in_progress, duration) {
                    Ok(()) => CommandResponse::ok(),
                    Err(e) => CommandResponse::error(e),
                }
            }
            Err(e) => CommandResponse::error(e),
        }
    }
}

impl CommandResponse {
    fn ok() -> Self {
        Self {
            status: "ok",
            message: None,
        }
    }

    fn error(message: String) -> Self {
        Self {
            status: "error",
            message: Some(message),
        }
    }
}

fn parse_command(line: &str) -> Result<ParsedCommand, String> {
    let request: CommandRequest =
        serde_json::from_str(line).map_err(|e| format!("invalid json: {}", e))?;

    match request.command.as_str() {
        "start_attack" => Ok(ParsedCommand::StartAttack),
        "stop_attack" => Ok(ParsedCommand::StopAttack),
        "pause" => {
            let duration_ms = request
                .duration_ms
                .ok_or_else(|| "pause requires duration_ms".to_string())?;
            Ok(ParsedCommand::Pause {
                duration: Duration::from_millis(duration_ms),
            })
        }
        command => Err(format!("unknown command: {}", command)),
    }
}

fn enqueue_pause(
    sender: &Sender<RuntimeCommand>,
    pause_in_progress: &AtomicBool,
    duration: Duration,
) -> Result<(), String> {
    if pause_in_progress
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return Err("pause already pending or running".to_string());
    }

    sender
        .send(RuntimeCommand::Pause { duration })
        .map_err(|e| {
            pause_in_progress.store(false, Ordering::SeqCst);
            format!("pause queue send failed: {}", e)
        })
}

fn remove_socket_if_exists(path: &Path) -> Result<(), std::io::Error> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::{enqueue_pause, parse_command, ParsedCommand, RuntimeCommand};
    use std::{
        sync::{
            atomic::{AtomicBool, Ordering},
            mpsc,
        },
        time::Duration,
    };

    #[test]
    fn parses_pause_command() {
        let command = parse_command(r#"{"command":"pause","duration_ms":250}"#).unwrap();

        assert!(matches!(
            command,
            ParsedCommand::Pause { duration } if duration == Duration::from_millis(250)
        ));
    }

    #[test]
    fn rejects_pause_without_duration() {
        let error = parse_command(r#"{"command":"pause"}"#).unwrap_err();

        assert!(error.contains("duration_ms"));
    }

    #[test]
    fn parses_attack_commands() {
        assert!(matches!(
            parse_command(r#"{"command":"start_attack"}"#).unwrap(),
            ParsedCommand::StartAttack
        ));
        assert!(matches!(
            parse_command(r#"{"command":"stop_attack"}"#).unwrap(),
            ParsedCommand::StopAttack
        ));
    }

    #[test]
    fn rejects_duplicate_pause_while_one_is_pending() {
        let (sender, receiver) = mpsc::channel();
        let pause_in_progress = AtomicBool::new(false);

        enqueue_pause(&sender, &pause_in_progress, Duration::from_millis(10)).unwrap();
        let error =
            enqueue_pause(&sender, &pause_in_progress, Duration::from_millis(20)).unwrap_err();

        assert!(error.contains("already pending"));
        assert!(pause_in_progress.load(Ordering::SeqCst));
        assert!(matches!(
            receiver.recv().unwrap(),
            RuntimeCommand::Pause { duration } if duration == Duration::from_millis(10)
        ));
    }
}
