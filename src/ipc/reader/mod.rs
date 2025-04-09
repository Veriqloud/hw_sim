pub mod errors;

use snafu::ResultExt;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::UnixStream,
};

use crate::{
    backend::BytesGenerator,
    ipc::{reader::errors::UnixStreamSnafu as IpcUnixStreamSnafu, Command},
};

use super::{
    super::errors::{
        BackendSnafu, Error as Hw_Sim_Error, IpcReaderSnafu, UnixStreamSnafu, WriterSnafu,
    },
    writer::actor::IPCWriterActorHandle,
};
// use super::UsbCommand;
use crate::backend::actor::ActorHandle as BackendHandle;

pub struct IPCReader {
    pub(in crate::ipc) cmd_stream: UnixStream,
    pub(in crate::ipc) writer_handle: IPCWriterActorHandle,
}

impl IPCReader {
    pub async fn read_cmd(&mut self) -> Result<Command, errors::Error> {
        match self.cmd_stream.read_u8().await {
            Ok(value) => {
                let cmd = match value {
                    0x26 => Command::Stop,
                    0x27 => Command::Start,
                    v => {
                        let reason = format!("Could not map the value {:x?} to Command.", v);
                        let e = errors::Error::Unexpected { reason };
                        return Err(e);
                    }
                };

                tracing::info!("Read Command: {:?}", &cmd);
                Ok(cmd)
            }
            Err(e) => {
                tracing::error!("Could not read command because {}", e);
                Err(e).context(IpcUnixStreamSnafu)
            }
        }
    }

    pub async fn process_cmd(&mut self, cmd: &Command) -> Result<(), Hw_Sim_Error> {
        match cmd {
            Command::Stop => self.writer_handle.stop().await.context(WriterSnafu),
            Command::Start => self.writer_handle.start().await.context(WriterSnafu),
        }
    }

    pub async fn new(cmd_stream: UnixStream, writer_handle: IPCWriterActorHandle) -> Self {
        IPCReader {
            cmd_stream,
            writer_handle,
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
