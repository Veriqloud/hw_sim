use snafu::ResultExt;
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};

use crate::simulator::Simulator;

use super::{
    errors::{self, Error},
    writer::{actor::ActorHandle as IpcWriterHandle, Writer},
    KeygenRequest,
};
use crate::simulator::actor::ActorHandle as SimulatorHandle;

pub struct IPCReader<S: Simulator, I: Writer, R: AsyncRead + Unpin> {
    pub(in crate::ipc) writer_handle: IpcWriterHandle<I>,
    pub(in crate::ipc) reader: BufReader<R>,
    pub(in crate::ipc) simulator_handle: SimulatorHandle<S>,
}

impl<S: Simulator, I: Writer, R: AsyncRead + Unpin> IPCReader<S, I, R> {
    pub async fn new(
        unix_stream: R,
        simulator_handle: SimulatorHandle<S>,
        ipc_writer_handle: IpcWriterHandle<I>,
    ) -> Result<Self, Error> {
        let reader = BufReader::new(unix_stream);

        Ok(IPCReader {
            simulator_handle,
            writer_handle: ipc_writer_handle,
            reader,
        })
    }

    pub async fn start(self) {
        let mut lines = self.reader.lines();

        loop {
            let line_opt = match lines.next_line().await {
                Ok(l) => l,
                Err(e) => {
                    tracing::error!("{}", e);
                    return;
                }
            };

            let line = match line_opt {
                Some(l) => l,
                None => {
                    tracing::warn!("read an empty line on the unix socket");
                    continue;
                }
            };

            let message = match serde_json::from_str::<KeygenRequest>(&line)
                .context(errors::SerdeJsonSnafu)
            {
                Ok(m) => m,
                Err(e) => {
                    tracing::error!("could not parse message: {}", e);
                    continue;
                }
            };

            match self
                .simulator_handle
                .generate_raw_keys(message.size, message.owner)
                .await
            {
                Ok(keys) => {
                    tracing::debug!("successfully generated {:?} keys ", keys);
                    match self.writer_handle.insert_keys(keys).await {
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
    }
}
