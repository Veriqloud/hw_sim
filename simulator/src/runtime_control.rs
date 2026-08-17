use std::{
    fs,
    io::{BufRead, BufReader, Write},
    os::unix::net::{UnixListener, UnixStream},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, Sender, TryRecvError},
        Arc,
    },
    thread,
    time::Duration,
};

use serde::{Deserialize, Serialize};
pub use sim_lib::simulation::GenerationProgress;

use crate::backend::actor::ActorHandle as SimulatorHandle;

pub struct RuntimeControl {
    receiver: Receiver<RuntimeCommand>,
    pause_in_progress: Arc<AtomicBool>,
}

impl RuntimeControl {
    pub fn try_recv(&self) -> Option<RuntimeCommand> {
        match self.receiver.try_recv() {
            Ok(command) => Some(command),
            Err(TryRecvError::Empty) | Err(TryRecvError::Disconnected) => None,
        }
    }

    pub fn complete_pause(&self) {
        self.pause_in_progress.store(false, Ordering::SeqCst);
    }

    pub fn recv(&self) -> Result<RuntimeCommand, mpsc::RecvError> {
        self.receiver.recv()
    }
}

pub type RuntimeReply<T> = Sender<Result<T, String>>;

#[derive(Debug)]
pub enum RuntimeCommand {
    Pause {
        duration: Duration,
        reply_to: RuntimeReply<GenerationProgress>,
    },
    Synchronize {
        batches_to_discard: u64,
        reply_to: RuntimeReply<()>,
    },
    Resume {
        reply_to: RuntimeReply<()>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum CommandRequest {
    StartAttack,
    StopAttack,
    Pause { duration_ms: u64 },
    Synchronize { batches_to_discard: u64 },
    Resume,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandResponse {
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub progress: Option<GenerationProgress>,
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
    let command_sender = sender.clone();
    let pause_in_progress = Arc::new(AtomicBool::new(false));
    let server_pause_in_progress = pause_in_progress.clone();

    thread::spawn(move || {
        tracing::info!(
            "Runtime control server listening on {}",
            socket_path.display()
        );
        for stream in listener.incoming() {
            match stream {
                Ok(stream) => {
                    handle_client(
                        stream,
                        &simulator_handle,
                        &command_sender,
                        &server_pause_in_progress,
                    );
                }
                Err(e) => tracing::error!("Runtime control socket accept failed: {}", e),
            }
        }
    });

    Ok(RuntimeControl {
        receiver,
        pause_in_progress,
    })
}

fn handle_client(
    stream: UnixStream,
    simulator_handle: &SimulatorHandle,
    command_sender: &Sender<RuntimeCommand>,
    pause_in_progress: &AtomicBool,
) {
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
            Ok(line) => handle_line(&line, simulator_handle, command_sender, pause_in_progress),
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

fn handle_line(
    line: &str,
    simulator_handle: &SimulatorHandle,
    command_sender: &Sender<RuntimeCommand>,
    pause_in_progress: &AtomicBool,
) -> CommandResponse {
    match serde_json::from_str::<CommandRequest>(line) {
        Ok(CommandRequest::StartAttack) => match simulator_handle.start_attack() {
            Ok(()) => CommandResponse::ok(),
            Err(e) => CommandResponse::error(format!("start_attack failed: {}", e)),
        },
        Ok(CommandRequest::StopAttack) => match simulator_handle.stop_attack() {
            Ok(()) => CommandResponse::ok(),
            Err(e) => CommandResponse::error(format!("stop_attack failed: {}", e)),
        },
        Ok(CommandRequest::Pause { duration_ms }) => enqueue_pause(
            command_sender,
            pause_in_progress,
            Duration::from_millis(duration_ms),
        ),
        Ok(CommandRequest::Synchronize { batches_to_discard }) => enqueue_recalibration_command(
            command_sender,
            pause_in_progress,
            "synchronize",
            |reply_to| RuntimeCommand::Synchronize {
                batches_to_discard,
                reply_to,
            },
        ),
        Ok(CommandRequest::Resume) => {
            enqueue_recalibration_command(command_sender, pause_in_progress, "resume", |reply_to| {
                RuntimeCommand::Resume { reply_to }
            })
        }
        Err(e) => CommandResponse::error(format!("invalid json: {}", e)),
    }
}

impl CommandResponse {
    fn ok() -> Self {
        Self {
            status: "ok".to_owned(),
            message: None,
            progress: None,
        }
    }

    fn progress(progress: GenerationProgress) -> Self {
        Self {
            status: "ok".to_owned(),
            message: None,
            progress: Some(progress),
        }
    }

    fn error(message: String) -> Self {
        Self {
            status: "error".to_owned(),
            message: Some(message),
            progress: None,
        }
    }
}

fn enqueue_pause(
    sender: &Sender<RuntimeCommand>,
    pause_in_progress: &AtomicBool,
    duration: Duration,
) -> CommandResponse {
    if pause_in_progress
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return CommandResponse::error("pause already pending or running".to_string());
    }

    match request_runtime(sender, |reply_to| RuntimeCommand::Pause {
        duration,
        reply_to,
    }) {
        Ok(progress) => CommandResponse::progress(progress),
        Err(error) => {
            pause_in_progress.store(false, Ordering::SeqCst);
            CommandResponse::error(error)
        }
    }
}

fn enqueue_recalibration_command(
    sender: &Sender<RuntimeCommand>,
    pause_in_progress: &AtomicBool,
    name: &'static str,
    command: impl FnOnce(RuntimeReply<()>) -> RuntimeCommand,
) -> CommandResponse {
    if !pause_in_progress.load(Ordering::SeqCst) {
        return CommandResponse::error(format!(
            "cannot {name} without a recalibration in progress"
        ));
    }

    match request_runtime(sender, command) {
        Ok(()) => CommandResponse::ok(),
        Err(error) => CommandResponse::error(error),
    }
}

fn request_runtime<T>(
    sender: &Sender<RuntimeCommand>,
    command: impl FnOnce(RuntimeReply<T>) -> RuntimeCommand,
) -> Result<T, String> {
    let (reply_to, reply) = mpsc::channel();
    sender
        .send(command(reply_to))
        .map_err(|error| format!("runtime command queue send failed: {error}"))?;
    reply
        .recv()
        .map_err(|error| format!("runtime command reply failed: {error}"))?
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
    use super::{
        enqueue_pause, CommandRequest, CommandResponse, GenerationProgress, RuntimeCommand,
    };
    use std::{
        sync::{
            atomic::{AtomicBool, Ordering},
            mpsc,
        },
        time::Duration,
    };

    #[test]
    fn parses_pause_command() {
        let command: CommandRequest =
            serde_json::from_str(r#"{"command":"pause","duration_ms":250}"#).unwrap();

        assert!(matches!(
            command,
            CommandRequest::Pause { duration_ms: 250 }
        ));
    }

    #[test]
    fn rejects_pause_without_duration() {
        let error = serde_json::from_str::<CommandRequest>(r#"{"command":"pause"}"#).unwrap_err();

        assert!(error.to_string().contains("duration_ms"));
    }

    #[test]
    fn parses_attack_commands() {
        assert!(matches!(
            serde_json::from_str::<CommandRequest>(r#"{"command":"start_attack"}"#).unwrap(),
            CommandRequest::StartAttack
        ));
        assert!(matches!(
            serde_json::from_str::<CommandRequest>(r#"{"command":"stop_attack"}"#).unwrap(),
            CommandRequest::StopAttack
        ));
    }

    #[test]
    fn control_messages_keep_their_wire_format() {
        assert_eq!(
            serde_json::to_string(&CommandRequest::StartAttack).unwrap(),
            r#"{"command":"start_attack"}"#
        );
        assert_eq!(
            serde_json::to_string(&CommandRequest::StopAttack).unwrap(),
            r#"{"command":"stop_attack"}"#
        );
        assert_eq!(
            serde_json::to_string(&CommandRequest::Pause { duration_ms: 250 }).unwrap(),
            r#"{"command":"pause","duration_ms":250}"#
        );
        assert_eq!(
            serde_json::to_string(&CommandRequest::Synchronize {
                batches_to_discard: 4,
            })
            .unwrap(),
            r#"{"command":"synchronize","batches_to_discard":4}"#
        );
        assert_eq!(
            serde_json::to_string(&CommandRequest::Resume).unwrap(),
            r#"{"command":"resume"}"#
        );

        assert_eq!(
            serde_json::to_string(&CommandResponse::ok()).unwrap(),
            r#"{"status":"ok"}"#
        );
        let response = CommandResponse::error("failed".to_owned());
        assert_eq!(
            serde_json::to_string(&response).unwrap(),
            r#"{"status":"error","message":"failed"}"#
        );
        assert_eq!(
            serde_json::from_str::<CommandResponse>(r#"{"status":"error","message":"failed"}"#)
                .unwrap(),
            response
        );
    }

    #[test]
    fn rejects_duplicate_pause_while_one_is_pending() {
        let (sender, receiver) = mpsc::channel();
        let pause_in_progress = AtomicBool::new(false);

        std::thread::scope(|scope| {
            let first = scope
                .spawn(|| enqueue_pause(&sender, &pause_in_progress, Duration::from_millis(10)));
            let RuntimeCommand::Pause { duration, reply_to } = receiver.recv().unwrap() else {
                panic!("expected pause command");
            };
            assert_eq!(duration, Duration::from_millis(10));

            let second = enqueue_pause(&sender, &pause_in_progress, Duration::from_millis(20));
            assert_eq!(second.status, "error");
            assert!(pause_in_progress.load(Ordering::SeqCst));

            let progress = GenerationProgress {
                event_count: 100,
                batch_pulse_count: 10,
            };
            reply_to.send(Ok(progress)).unwrap();
            assert_eq!(first.join().unwrap().progress, Some(progress));
        });
    }
}
