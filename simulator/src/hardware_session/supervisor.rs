use std::{
    fs::{File, OpenOptions},
    io,
    thread::{self, sleep},
    time::Duration,
};

use configs::ipc::{AliceIpcConfig, BobIpcConfig, Configuration as IpcConfiguration};
use sim_lib::hardware::modes::SimulatorMode;
use snafu::Snafu;

use super::{HardwareSessionRunner, SessionExit};
use crate::{
    backend::actor::ActorHandle as SimulatorHandle,
    ipc::{
        fifo_connection::FifoConnection,
        writer::{actor::IPCWriterActorHandle, errors::Error as WriterError},
    },
    runtime_control::RuntimeControl,
    runtime_status::RuntimeStatusFiles,
};

const SESSION_RETRY_DELAY: Duration = Duration::from_millis(500);
const SESSION_RESTART_DELAY: Duration = Duration::from_millis(250);

#[derive(Debug, Snafu)]
enum FifoConnectionError {
    #[snafu(display("Failed to open {name} at '{path}': {source}"))]
    Open {
        name: &'static str,
        path: String,
        source: io::Error,
    },
    #[snafu(display("Failed to initialize FIFO connection: {source}"))]
    Initialize { source: WriterError },
}

#[derive(Debug, Snafu)]
enum RecalibrationError {
    #[snafu(display("Failed to begin runtime recalibration: {source}"))]
    Begin { source: io::Error },
    #[snafu(display("Failed while waiting for node idle: {source}"))]
    WaitForNodeIdle { source: io::Error },
    #[snafu(display("Failed to reset IPC FIFOs after runtime pause: {source}"))]
    ResetFifos { source: configs::ipc::errors::Error },
    #[snafu(display("Failed to recreate qkd ready file after runtime pause: {source}"))]
    CreateReady { source: io::Error },
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum FifoPathsState {
    ReadyForOpen,
    ResetRequired,
}

pub struct HardwareSessionSupervisor<'config> {
    ipc_config: &'config IpcConfiguration,
    simulator_handle: SimulatorHandle,
    writer_handle: IPCWriterActorHandle,
    runtime_control: RuntimeControl,
    runtime_status: RuntimeStatusFiles,
}

impl<'config> HardwareSessionSupervisor<'config> {
    pub fn new(
        ipc_config: &'config IpcConfiguration,
        simulator_handle: SimulatorHandle,
        runtime_control: RuntimeControl,
        runtime_status: RuntimeStatusFiles,
    ) -> Self {
        Self {
            ipc_config,
            simulator_handle,
            writer_handle: IPCWriterActorHandle::new(),
            runtime_control,
            runtime_status,
        }
    }

    pub fn run(self) {
        let mut fifo_paths_state = FifoPathsState::ReadyForOpen;

        loop {
            let (role, command_path, simulator_mode) = self.session_parameters();
            tracing::info!("{} workflow: Waiting for a controller...", role);

            if fifo_paths_state == FifoPathsState::ResetRequired {
                tracing::info!("Resetting FIFOs for new {} session.", role);
                if let Err(error) = self.ipc_config.reset_ipc_fifos() {
                    tracing::error!(
                        "Failed to reset FIFOs for {}: {}. Retrying in {:?}.",
                        role,
                        error,
                        SESSION_RETRY_DELAY
                    );
                    sleep(SESSION_RETRY_DELAY);
                    continue;
                }
            }

            let fifo_connection = match self.open_fifo_connection() {
                Ok(connection) => connection,
                Err(error @ FifoConnectionError::Open { .. }) => {
                    tracing::error!("{}. Retrying in {:?}.", error, SESSION_RETRY_DELAY);
                    fifo_paths_state = FifoPathsState::ResetRequired;
                    sleep(SESSION_RETRY_DELAY);
                    continue;
                }
                Err(error) => {
                    tracing::error!("{} {}", role, error);
                    return;
                }
            };

            let mut session_runner = HardwareSessionRunner::new(
                command_path.to_owned(),
                fifo_connection,
                self.simulator_handle.clone(),
                simulator_mode,
                &self.runtime_control,
            );

            tracing::info!("Starting hardware session for {}.", role);
            match session_runner.run() {
                Ok(SessionExit::RecalibrationRequested { duration }) => {
                    tracing::info!("Starting runtime recalibration pause for {:?}.", duration);
                    let begin_result = self
                        .runtime_status
                        .begin_recalibration()
                        .map_err(|source| RecalibrationError::Begin { source });

                    drop(session_runner);

                    let recalibration_result =
                        begin_result.and_then(|()| self.finish_runtime_pause(duration));
                    self.runtime_control.complete_pause();

                    if let Err(error) = recalibration_result {
                        tracing::error!("{}", error);
                        return;
                    }
                    fifo_paths_state = FifoPathsState::ReadyForOpen;
                }
                Ok(SessionExit::Stopped) => {
                    drop(session_runner);
                    tracing::info!("Hardware session for {} stopped cleanly.", role);
                    fifo_paths_state = FifoPathsState::ResetRequired;
                }
                Ok(SessionExit::PeerDisconnected) => {
                    drop(session_runner);
                    tracing::info!("Hardware session peer for {} disconnected.", role);
                    fifo_paths_state = FifoPathsState::ResetRequired;
                }
                Err(error) => {
                    drop(session_runner);
                    tracing::error!(
                        "Hardware session for {} ended with an error: {:?}. Preparing for new connection.",
                        role,
                        error
                    );
                    fifo_paths_state = FifoPathsState::ResetRequired;
                }
            }
            sleep(SESSION_RESTART_DELAY);
        }
    }

    fn session_parameters(&self) -> (&'static str, &str, SimulatorMode) {
        match self.ipc_config {
            IpcConfiguration::Alice(config) => {
                ("Alice", &config.command_path, SimulatorMode::Source)
            }
            IpcConfiguration::Bob(config) => ("Bob", &config.command_path, SimulatorMode::Detector),
        }
    }

    fn open_fifo_connection(&self) -> Result<FifoConnection, FifoConnectionError> {
        match self.ipc_config {
            IpcConfiguration::Alice(config) => self.open_alice_fifo_connection(config),
            IpcConfiguration::Bob(config) => self.open_bob_fifo_connection(config),
        }
    }

    fn open_alice_fifo_connection(
        &self,
        config: &AliceIpcConfig,
    ) -> Result<FifoConnection, FifoConnectionError> {
        let gcr_file = OpenOptions::new()
            .write(true)
            .open("/dev/null")
            .map_err(|source| FifoConnectionError::Open {
                name: "Alice GCR sink",
                path: "/dev/null".to_owned(),
                source,
            })?;

        let (angles_result, gc_read_result) = thread::scope(|scope| {
            let angles =
                scope.spawn(|| OpenOptions::new().write(true).open(&config.angle_file_path));
            let gc_read = scope.spawn(|| {
                OpenOptions::new()
                    .read(true)
                    .open(&config.gc_read_file_path)
            });
            (angles.join().unwrap(), gc_read.join().unwrap())
        });

        let angles_file = open_result(angles_result, "Alice angles FIFO", &config.angle_file_path)?;
        let gc_read_file = open_result(
            gc_read_result,
            "Alice GC reader FIFO",
            &config.gc_read_file_path,
        )?;

        FifoConnection::new(
            gc_read_file,
            self.writer_handle.clone(),
            gcr_file,
            angles_file,
        )
        .map_err(|source| FifoConnectionError::Initialize { source })
    }

    fn open_bob_fifo_connection(
        &self,
        config: &BobIpcConfig,
    ) -> Result<FifoConnection, FifoConnectionError> {
        let (angles_result, gcr_result, gc_read_result) = thread::scope(|scope| {
            let angles =
                scope.spawn(|| OpenOptions::new().write(true).open(&config.angle_file_path));
            let gcr = scope.spawn(|| OpenOptions::new().write(true).open(&config.gcr_file_path));
            let gc_read = scope.spawn(|| {
                OpenOptions::new()
                    .read(true)
                    .open(&config.gc_read_file_path)
            });
            (
                angles.join().unwrap(),
                gcr.join().unwrap(),
                gc_read.join().unwrap(),
            )
        });

        let angles_file = open_result(angles_result, "Bob angles FIFO", &config.angle_file_path)?;
        let gcr_file = open_result(gcr_result, "Bob GCR FIFO", &config.gcr_file_path)?;
        let gc_read_file = open_result(
            gc_read_result,
            "Bob GC reader FIFO",
            &config.gc_read_file_path,
        )?;

        FifoConnection::new(
            gc_read_file,
            self.writer_handle.clone(),
            gcr_file,
            angles_file,
        )
        .map_err(|source| FifoConnectionError::Initialize { source })
    }

    fn finish_runtime_pause(&self, duration: Duration) -> Result<(), RecalibrationError> {
        tracing::info!("Waiting for node idle file before recalibration pause.");
        self.runtime_status
            .wait_for_node_idle()
            .map_err(|source| RecalibrationError::WaitForNodeIdle { source })?;

        tracing::info!("Node idle detected. Sleeping for requested pause duration.");
        sleep(duration);

        self.ipc_config
            .reset_ipc_fifos()
            .map_err(|source| RecalibrationError::ResetFifos { source })?;
        self.runtime_status
            .create_qkd_ready()
            .map_err(|source| RecalibrationError::CreateReady { source })?;

        tracing::info!("Runtime recalibration pause completed.");
        Ok(())
    }
}

fn open_result(
    result: Result<File, io::Error>,
    name: &'static str,
    path: &str,
) -> Result<File, FifoConnectionError> {
    result.map_err(|source| FifoConnectionError::Open {
        name,
        path: path.to_owned(),
        source,
    })
}
