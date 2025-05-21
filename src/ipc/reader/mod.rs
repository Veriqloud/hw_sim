pub mod errors;

use snafu::ResultExt;
use tokio::{fs::File, io::AsyncReadExt};

use crate::{backend::actor::ActorHandle as SimulatorHandle, ipc::Command};

// use super::super::errors::{Error as Hw_Sim_Error, WriterSnafu}; // Hw_Sim_Error is crate::errors::Error

use super::writer::actor::IPCWriterActorHandle;

pub struct IPCReader {
    pub(in crate::ipc) cmd_file: File,
    pub(in crate::ipc) gc_file: File,
    pub(in crate::ipc) writer_handle: IPCWriterActorHandle,
    pub(in crate::ipc) simulator_handle: SimulatorHandle,
}

impl IPCReader {
    pub async fn read_cmd(&mut self) -> Result<Command, errors::Error> {
        match self.cmd_file.read_u8().await {
            Ok(value) => {
                let cmd = match value {
                    0x26 => Command::Stop,
                    0x27 => Command::Start,
                    v => {
                        let reason = format!("Could not map the value {:#04x} to Command.", v);
                        tracing::warn!("{}", &reason);
                        return Err(errors::Error::Unexpected { reason });
                    }
                };

                tracing::info!("Read Command: {:?} ({:04x}) to Command.", &cmd, value);
                Ok(cmd)
            }
            Err(e) => {
                tracing::error!("Could not read command because {}", e);
                Err(e).context(crate::ipc::reader::errors::CommandFileIoSnafu)
            }
        }
    }

    async fn read_gc_from_file(&mut self) -> Result<u64, errors::Error> {
        // Assuming GC is stored as a little-endian u64.
        self.gc_file.read_u64_le().await.map_err(|e| {
            let reason = format!("Failed to read GC from file: {}", e);
            tracing::error!("{}", &reason);
            errors::Error::Unexpected { reason }
        })
    }

    pub fn new(
        cmd_file: File,
        gc_file: File,
        simulator_handle: SimulatorHandle,
        writer_handle: IPCWriterActorHandle,
    ) -> Self {
        IPCReader {
            writer_handle,
            cmd_file,
            gc_file,
            simulator_handle,
        }
    }

    pub async fn start(mut self) -> Result<(), errors::Error> {
        loop {
            let cmd = self.read_cmd().await?;
            tracing::info!("Processing command: {:?}", &cmd);

            match cmd {
                Command::Start => {
                    self.simulator_handle.start().await.map_err(|e| {
                        errors::Error::Unexpected {
                            reason: format!("Simulator start command failed: {}", e),
                        }
                    })?;
                    tracing::info!("Simulator acknowledged start. Reading GC...");

                    let gc = self.read_gc_from_file().await?;
                    tracing::info!("Read GC: {}. Seeding simulator...", gc);

                    self.simulator_handle
                        .seed_and_start_generation(gc)
                        .await
                        .map_err(|e| errors::Error::Unexpected {
                            reason: format!("Simulator seed_and_start_generation failed: {}", e),
                        })?;
                    tracing::info!("Simulator seeded and generation started. Starting writer...");

                    self.writer_handle.start().await.map_err(|e| {
                        errors::Error::Unexpected {
                            reason: format!("IPC Writer start failed: {}", e),
                        }
                    })?;
                    tracing::info!("IPC Writer started.");
                }
                Command::Stop => {
                    tracing::info!("Stopping IPC Writer...");
                    self.writer_handle.stop().await.map_err(|e| {
                        errors::Error::Unexpected {
                            reason: format!("IPC Writer stop failed: {}", e),
                        }
                    })?;
                    tracing::info!("IPC Writer stopped. Stopping simulator...");

                    self.simulator_handle.stop().await.map_err(|e| {
                        errors::Error::Unexpected {
                            reason: format!("Simulator stop command failed: {}", e),
                        }
                    })?;
                    tracing::info!("Simulator stopped.");
                }
            }
            tracing::info!("Successfully processed command: {:?}", &cmd);
        }
    }
}
