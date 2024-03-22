pub mod errors;

use snafu::ResultExt;
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader, BufWriter},
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
        let mut writer = BufWriter::new(write_half);
        let mut reader = reader.lines();
        tracing::info!("IPC running");
        if let Some(line) = reader.next_line().await.unwrap() {
            match serde_json::from_str(&line).context(errors::SerdeJsonSnafu) {
                Ok(msg) => match msg {
                    UsbCommand::Ok => {
                        tracing::error!("Message not expected !");
                    }
                    UsbCommand::FifoIdle => match self.backend_handle.fifo_idle().await {
                        Ok(_) => {
                            tracing::debug!("Sucessfully turn the Simulator into Idle.");
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
                                tracing::debug!("Successfully started at GC = {}", gc);
                                writer.write_all(&UsbCommand::Ok.as_bytes()).await.unwrap();
                            }
                            Err(e) => {
                                tracing::error!("{}", e);
                                writer.write_all(&UsbCommand::KO.as_bytes()).await.unwrap();
                            }
                        }
                    }
                    UsbCommand::ReadAngles => {
                        match self.backend_handle.read_angles().await {
                            Ok(data) => {
                                tracing::debug!("successfully generated {:?} bytes", data.len());
                                match writer.write_all(&data).await {
                                    Ok(_) => {
                                        writer.flush().await.unwrap();
                                        tracing::debug!("successfully inserted bytes");
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
                                tracing::debug!("global counter: {:?}", v);
                                match writer.write_u64(v.unwrap_or(0)).await {
                                    Ok(_) => {
                                        tracing::debug!("successfully inserted bytes");
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
                        }
                    }
                    UsbCommand::AngleSet { angles } => {
                        match self.backend_handle.set_angles(angles).await {
                            Ok(_) => {
                                tracing::debug!("Successfully set angles to : {:?}", &angles);
                                match writer.write(&UsbCommand::Ok.as_bytes()).await {
                                    Ok(_) => tracing::debug!("Send OK"),
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
                                tracing::debug!(
                                    "Successfully set role with nb_of_parties {} and position {}",
                                    number_of_parties,
                                    position
                                );
                                match writer.write(&UsbCommand::Ok.as_bytes()).await {
                                    Ok(_) => tracing::debug!("Send OK"),
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
