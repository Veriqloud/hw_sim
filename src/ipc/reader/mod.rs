pub mod errors;

use snafu::ResultExt;
use tokio::{fs::File, io::AsyncReadExt};

use crate::{backend::actor::ActorHandle as SimulatorHandle, ipc::Command};

use super::super::errors::{Error as Hw_Sim_Error, WriterSnafu};

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

    async fn read_gc_from_file(&mut self) -> Result<u64, Hw_Sim_Error> {
        // Assuming GC is stored as a little-endian u64.
        let gc = self.gc_file.read_u64_le().await.unwrap();
        Ok(gc)
    }

    pub async fn process_cmd(&mut self, cmd: &Command) -> Result<(), Hw_Sim_Error> {
        match cmd {
            Command::Stop => {
                tracing::info!("Writer handle will send Stop message");
                self.writer_handle.stop().await.context(WriterSnafu)?;
                tracing::info!("Writer handle send stop");
                Ok(())
            }
            Command::Start => self.writer_handle.start().await.context(WriterSnafu),
        }
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
            let cmd = match self.read_cmd().await {
                Ok(c) => c,
                Err(e) => return Err(e),
            };
            match self.process_cmd(&cmd).await {
                Ok(_) => {
                    tracing::info!("Processing of {:?}: Success", &cmd);
                }
                Err(_e) => {
                    tracing::error!("Processing of {:?}: Failure", &cmd);
                }
            }
        }
    }
}
