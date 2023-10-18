pub mod errors;

use snafu::ResultExt;
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};

use crate::backend::BytesGenerator;

use super::{
    writer::{actor::ActorHandle as IpcWriterHandle, Writer},
    Message,
};
use crate::backend::actor::ActorHandle as BackendHandle;
use errors::Error;

pub struct IPCReader<S: BytesGenerator, I: Writer, R: AsyncRead + Unpin> {
    pub(in crate::ipc) writer_handle: IpcWriterHandle<I>,
    pub(in crate::ipc) reader: BufReader<R>,
    pub(in crate::ipc) backend_handle: BackendHandle<S>,
}

impl<S: BytesGenerator, I: Writer, R: AsyncRead + Unpin> IPCReader<S, I, R> {
    pub async fn new(
        unix_stream: R,
        backend_handle: BackendHandle<S>,
        ipc_writer_handle: IpcWriterHandle<I>,
    ) -> Result<Self, Error> {
        let reader = BufReader::new(unix_stream);

        Ok(IPCReader {
            backend_handle,
            writer_handle: ipc_writer_handle,
            reader,
        })
    }

    pub async fn start(self) {
        let mut reader = self.reader.lines();
        if let Some(line) = reader.next_line().await.unwrap() {
            match serde_json::from_str(&line).context(errors::SerdeJsonSnafu) {
                Ok(msg) => match msg {
                    Message::ReadAnglesRequest => {
                        match self.backend_handle.read_angles().await {
                            Ok(data) => {
                                tracing::debug!("successfully generated {:?} bytes", data);
                                match self.writer_handle.insert_data(data).await {
                                    Ok(_) => {
                                        tracing::debug!("successfully inserted keys");
                                        println!("successfully inserted keys");
                                    }
                                    Err(e) => {
                                        tracing::error!("{}", e)
                                    }
                                }
                            }
                            Err(e) => {
                                tracing::error!("{}", e)
                            }
                        };
                    }
                    Message::GetGlobalCounter => todo!(),
                    Message::GetGcSafe => todo!(),
                    Message::SetModulatorState => todo!(),
                },
                Err(e) => tracing::error!("{}", e),
            };
        }
    }
}
