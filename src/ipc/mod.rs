pub(crate) mod errors;

use serde::Deserialize;
use serde::Serialize;
use snafu::ResultExt;
use tokio::io::AsyncBufReadExt;
use tokio::io::AsyncRead;
use tokio::io::BufReader;

use crate::insertor::{actor::ActorHandle as InsertorHandle, Insertor};
use crate::simulator::{actor::ActorHandle as SimuHandle, errors::Error, Simulator};

pub struct IPCReader<S: Simulator, I: Insertor, R: AsyncRead + Unpin> {
    insertor_handle: InsertorHandle<I>,
    reader: BufReader<R>,
    simulator_handle: SimuHandle<S>,
}

impl<S: Simulator, I: Insertor, R: AsyncRead + Unpin> IPCReader<S, I, R> {
    pub async fn new(
        unix_stream: R,
        simulator_handle: SimuHandle<S>,
        insertor_handle: InsertorHandle<I>,
    ) -> Result<Self, Error> {
        let reader = BufReader::new(unix_stream);

        Ok(IPCReader {
            simulator_handle,
            insertor_handle,
            reader,
        })
    }

    pub async fn start(self) {
        let mut lines = self.reader.lines();

        loop {
            let line_res = lines.next_line().await;

            if line_res.is_err() {
                tracing::error!("{}", line_res.unwrap_err());
                return;
            }

            let line_res = line_res.unwrap();

            // If this is `None`, it means the iterator is exhausted.
            if line_res.is_none() {
                tracing::warn!("read an empty line on the unix socket");
                return;
            }

            let line = line_res.unwrap();
            let message_res =
                serde_json::from_str::<LoadFifo>(&line).context(errors::SerdeJsonSnafu);

            if message_res.is_err() {
                tracing::error!("could not parse message: {}", message_res.unwrap_err());
                continue;
            }
            let msg = message_res.unwrap();

            match self
                .simulator_handle
                .generate_raw_keys(msg.size, msg.owner)
                .await
            {
                Ok(keys) => {
                    tracing::debug!("successfully generated {:?} keys ", keys);
                    println!("successfully generated {:?} keys ", keys);
                    match self.insertor_handle.insert_keys(keys).await {
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

#[derive(Serialize, Deserialize, Debug)]
pub struct LoadFifo {
    pub size: usize,
    pub owner: String,
}
