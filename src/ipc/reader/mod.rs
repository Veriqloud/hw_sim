pub mod errors;

use snafu::ResultExt;
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::UnixStream,
};

use crate::backend::BytesGenerator;

use super::UsbCommand;
use crate::backend::actor::ActorHandle as BackendHandle;

pub struct IPCReader<G: BytesGenerator> {
    pub(in crate::ipc) stream: UnixStream,
    pub(in crate::ipc) backend_handle: BackendHandle<G>,
}

impl<G: BytesGenerator> IPCReader<G> {
    pub async fn new(unix_stream: UnixStream, backend_handle: BackendHandle<G>) -> Self {
        IPCReader {
            backend_handle,
            stream: unix_stream,
        }
    }

    pub async fn start(mut self) {
        let (read_half, write_half) = self.stream.split();
        let reader = BufReader::new(read_half);
        let mut writer = write_half;
        let mut reader = reader.lines();
        tracing::info!("New IPC server running");
        loop {
            let line_res = reader.next_line().await;

            if line_res.is_err() {
                tracing::error!("{}", line_res.unwrap_err());
                return;
            }
            let line_res = line_res.unwrap();

            if line_res.is_none() {
                tracing::warn!("Read an empty line on the unix socket");
                return;
            }

            let line = line_res.unwrap();

            match serde_json::from_str(&line).context(errors::SerdeJsonSnafu) {
                Ok(msg) => match msg {
                    UsbCommand::Ok => {
                        tracing::error!("Message not expected !");
                    }
                    UsbCommand::FifoIdle => match self.backend_handle.fifo_idle().await {
                        Ok(_) => {
                            tracing::info!("Successfully turn the Simulator into Idle.");
                            writer.write_all(&UsbCommand::Ok.as_bytes()).await.unwrap();
                        }
                        Err(e) => {
                            tracing::error!("{}", e);
                            writer.write_all(&UsbCommand::KO.as_bytes()).await.unwrap();
                        }
                    },
                    UsbCommand::StartAtGc { gc } => {
                        // Read expected for Global_counter value (u64)
                        match self.backend_handle.start_at_gc(gc).await {
                            Ok(_) => {
                                tracing::info!("Successfully started at GC = {}", gc);
                                tracing::info!("Writing {:?}", &UsbCommand::Ok.as_bytes());
                                // let a = writer.write(b"some bytes").await.unwrap();
                                let a = writer.write(&UsbCommand::Ok.as_bytes()).await.unwrap();
                                tracing::info!("Write {a} bytes done, flush next");
                                writer.flush().await.unwrap();
                                tracing::info!("Flush done");
                            }
                            Err(e) => {
                                tracing::error!("{}", e);
                                writer.write_all(&UsbCommand::KO.as_bytes()).await.unwrap();
                            }
                        }
                    }
                    UsbCommand::ReadAngles => {
                        tracing::info!("Processing ReadAngle request...");
                        match self.backend_handle.read_angles().await {
                            Ok(data) => {
                                tracing::info!("successfully generated {:?} bytes", data.len());
                                match writer.write_all(&data).await {
                                    Ok(_) => {
                                        writer.flush().await.unwrap();
                                        tracing::info!("successfully inserted bytes");
                                    }
                                    Err(e) => {
                                        tracing::error!("{}", e)
                                    }
                                }
                            }
                            Err(e) => {
                                tracing::error!("{}", e);
                                match writer.write(&UsbCommand::KO.as_bytes()).await {
                                    Ok(_) => {
                                        tracing::debug!("Send KO response");
                                    }
                                    Err(e) => {
                                        tracing::error!("{}", e)
                                    }
                                }
                            }
                        };
                    }
                    UsbCommand::GetCurrentGc => {
                        match self.backend_handle.get_global_counter().await {
                            Ok(v) => {
                                tracing::info!("global counter: {:?}", v);
                                match writer.write_u64(v.unwrap_or(0)).await {
                                    Ok(_) => {
                                        tracing::info!("successfully inserted bytes");
                                    }
                                    Err(e) => {
                                        tracing::error!("{}", e)
                                    }
                                }
                            }

                            Err(e) => {
                                tracing::error!("{}", e);
                                match writer.write(&UsbCommand::KO.as_bytes()).await {
                                    Ok(_) => {
                                        tracing::info!("Send KO response");
                                    }
                                    Err(e) => {
                                        tracing::error!("{}", e)
                                    }
                                }
                            }
                        }
                    }
                    UsbCommand::AngleSet { angles } => {
                        match self.backend_handle.set_angles(angles).await {
                            Ok(_) => {
                                tracing::info!("Successfully set angles to : {:?}", &angles);
                                match writer.write(&UsbCommand::Ok.as_bytes()).await {
                                    Ok(_) => tracing::info!("Send OK"),
                                    Err(e) => tracing::error!("{}", e),
                                }
                            }
                            Err(e) => tracing::error!("{}", e),
                        }
                    }
                    UsbCommand::KO => {
                        tracing::error!("Message not expected !");
                    }
                    UsbCommand::SetRole {
                        number_of_parties,
                        position,
                    } => {
                        match self
                            .backend_handle
                            .set_role(number_of_parties, position)
                            .await
                        {
                            Ok(_) => {
                                tracing::info!(
                                    "Successfully set role with nb_of_parties {} and position {}",
                                    number_of_parties,
                                    position
                                );
                                match writer.write(&UsbCommand::Ok.as_bytes()).await {
                                    Ok(_) => tracing::info!("Send OK"),
                                    Err(e) => tracing::error!("{}", e),
                                }
                            }
                            Err(e) => tracing::error!("{}", e),
                        }
                    }
                },
                Err(e) => tracing::error!("{}", e),
            };
        }
    }
}
