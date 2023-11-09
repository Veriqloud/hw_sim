pub mod errors;

use libhardware::ModulatorState;
use snafu::ResultExt;
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader, BufWriter},
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
        if let Some(line) = reader.next_line().await.unwrap() {
            match serde_json::from_str(&line).context(errors::SerdeJsonSnafu) {
                Ok(msg) => match msg {
                    UsbCommand::Ok => {
                        tracing::error!("Message not expected !");
                    }
                    UsbCommand::FifoIdle => {
                        let gc = self
                            .backend_handle
                            .get_global_counter()
                            .await
                            .unwrap()
                            .unwrap_or(0_u64);
                        match self
                            .backend_handle
                            .set_modulator_state(gc, ModulatorState::Idle)
                            .await
                        {
                            Ok(_v) => {
                                let resp = UsbCommand::Ok.as_bytes();
                                match writer.write_u8(resp).await {
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
                                let resp = UsbCommand::KO.as_bytes();
                                match writer.write_u8(resp).await {
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
                    UsbCommand::StartAtGc => todo!(),
                    UsbCommand::ReadAngles => {
                        let mut buf = [0_u8; 8];
                        reader.get_mut().read_exact(&mut buf).await.unwrap();
                        match self.backend_handle.read_angles().await {
                            Ok(data) => {
                                tracing::debug!("successfully generated {:?} bytes", data.len());
                                match writer.write_all(&data).await {
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
                                let resp = UsbCommand::KO.as_bytes();
                                match writer.write_u8(resp).await {
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
                    UsbCommand::GetCurrentGc => match self.backend_handle.get_gc_safe().await {
                        Ok(v) => {
                            tracing::debug!("global counter: {:?}", v);
                            match writer.write_u64(v).await {
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
                            let resp = UsbCommand::KO.as_bytes();
                            match writer.write_u8(resp).await {
                                Ok(_) => {
                                    tracing::debug!("Send KO response");
                                }
                                Err(e) => {
                                    tracing::error!("{}", e)
                                }
                            }
                        }
                    },
                    UsbCommand::AngleSet => todo!(),
                    UsbCommand::KO => {
                        tracing::error!("Message not expected !");
                    }
                },
                Err(e) => tracing::error!("{}", e),
            };
        }
    }
}
