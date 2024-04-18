pub mod errors;

use snafu::ResultExt;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::UnixStream,
};

use crate::{
    backend::BytesGenerator,
    ipc::{reader::errors::UnixStreamSnafu as IpcUnixStreamSnafu, RawCommand},
};

use super::super::errors::{BackendSnafu, Error as Hw_Sim_Error, IpcReaderSnafu, UnixStreamSnafu};
use super::UsbCommand;
use crate::backend::actor::ActorHandle as BackendHandle;

pub struct IPCReader<G: BytesGenerator> {
    pub(in crate::ipc) stream: UnixStream,
    pub(in crate::ipc) backend_handle: BackendHandle<G>,
}

impl<G: BytesGenerator> IPCReader<G> {
    pub async fn read_cmd(&mut self) -> Result<UsbCommand, errors::Error> {
        match self.stream.read_u8().await {
            Ok(value) => {
                tracing::info!("Read {:?}", value);
                let raw_cmd = match value {
                    0x16 => RawCommand::Ok,
                    0x26 => RawCommand::FifosIdle,
                    0x27 => RawCommand::StartAtGc,
                    0x28 => RawCommand::ReadAngles,
                    0x29 => RawCommand::GetCurrentGc,
                    0x2a => RawCommand::AngleSet,
                    0xaa => RawCommand::Ko,
                    0xab => RawCommand::SetRole,
                    v => {
                        let reason = format!("Could not map the value {:x?} to RawCommand.", v);
                        let e = errors::Error::Unexpected { reason };
                        return Err(e);
                    }
                };
                let usb_cmd = match raw_cmd {
                    RawCommand::Ok => UsbCommand::Ok,
                    RawCommand::FifosIdle => UsbCommand::FifoIdle,
                    RawCommand::Ko => UsbCommand::KO,
                    RawCommand::StartAtGc => {
                        let gc = self
                            .stream
                            .read_u64()
                            .await
                            .context(UnixStreamSnafu)
                            .unwrap();
                        UsbCommand::StartAtGc { gc }
                    }
                    RawCommand::GetCurrentGc => UsbCommand::GetCurrentGc,
                    RawCommand::ReadAngles => UsbCommand::ReadAngles,
                    RawCommand::AngleSet => {
                        let mut angles = [0u8; 4];
                        let _size = self
                            .stream
                            .read_exact(&mut angles)
                            .await
                            .context(UnixStreamSnafu)
                            .unwrap();
                        UsbCommand::AngleSet { angles }
                    }
                    RawCommand::SetRole => {
                        let out = self.stream.read_u64().await.unwrap();
                        let (h, l) = ((out >> 32) as u32, out as u32);
                        UsbCommand::SetRole {
                            number_of_parties: h,
                            position: l,
                        }
                    }
                };

                tracing::info!("Command: {:?}", &usb_cmd);
                Ok(usb_cmd)
            }
            Err(e) => {
                tracing::error!("Could not read command because {}", e);
                Err(e).context(IpcUnixStreamSnafu)
            }
        }
    }

    pub async fn process_cmd(&mut self, cmd: &UsbCommand) -> Result<(), Hw_Sim_Error> {
        match cmd {
            UsbCommand::Ok => {
                tracing::error!("Message not expected !");
                let err = Err(errors::Error::Unexpected {
                    reason: "Unexpected message received".to_string(),
                });
                err.context(IpcReaderSnafu)
            }
            UsbCommand::FifoIdle => match self.backend_handle.fifo_idle().await {
                Ok(_) => {
                    tracing::info!("Successfully turn the Simulator into Idle.");
                    match self.stream.write_all(&UsbCommand::Ok.as_bytes()).await {
                        Ok(_) => Ok(()),
                        Err(e) => Err(e).context(UnixStreamSnafu),
                    }
                }
                Err(e) => {
                    tracing::error!("{}", &e);
                    match self.stream.write_all(&UsbCommand::KO.as_bytes()).await {
                        Ok(_) => Err(e).context(BackendSnafu),
                        Err(e) => Err(e).context(UnixStreamSnafu),
                    }
                }
            },
            UsbCommand::StartAtGc { gc } => {
                // Read expected for Global_counter value (u64)
                match self.backend_handle.start_at_gc(*gc).await {
                    Ok(_) => {
                        tracing::info!("Successfully started at GC = {}", gc);
                        tracing::info!("Writing {:?}", &UsbCommand::Ok.as_bytes());
                        match self.stream.write(&UsbCommand::Ok.as_bytes()).await {
                            Ok(a) => {
                                tracing::info!("Write {a} bytes done, flush next");
                                Ok(())
                            }
                            Err(e) => {
                                tracing::error!("{}", e);
                                Err(e).context(UnixStreamSnafu)
                            }
                        }
                    }
                    Err(e) => {
                        tracing::error!("{}", e);
                        match self.stream.write_all(&UsbCommand::KO.as_bytes()).await {
                            Ok(_) => Err(e).context(BackendSnafu),
                            Err(e) => Err(e).context(UnixStreamSnafu),
                        }
                    }
                }
            }
            UsbCommand::ReadAngles => {
                tracing::info!("Processing ReadAngle request...");
                match self.backend_handle.read_angles().await {
                    Ok(data) => {
                        tracing::info!("successfully generated {:?} bytes", data.len());
                        match self.stream.write_all(&data).await {
                            Ok(_) => {
                                tracing::info!("successfully inserted bytes");
                                Ok(())
                            }
                            Err(e) => {
                                tracing::error!("{}", e);
                                Err(e).context(UnixStreamSnafu)
                            }
                        }
                    }
                    Err(e) => {
                        tracing::error!("{}", e);
                        match self.stream.write(&UsbCommand::KO.as_bytes()).await {
                            Ok(_) => {
                                tracing::debug!("Send KO response");
                                Err(e).context(BackendSnafu)
                            }
                            Err(e) => {
                                tracing::error!("{}", e);
                                Err(e).context(UnixStreamSnafu)
                            }
                        }
                    }
                }
            }
            UsbCommand::GetCurrentGc => match self.backend_handle.get_global_counter().await {
                Ok(v) => {
                    tracing::info!("global counter: {:?}", v);
                    match self.stream.write_u64(v.unwrap_or(0)).await {
                        Ok(_) => {
                            tracing::info!("successfully inserted bytes");
                            Ok(())
                        }
                        Err(e) => {
                            tracing::error!("{}", e);
                            Err(e).context(UnixStreamSnafu)
                        }
                    }
                }

                Err(e) => {
                    tracing::error!("{}", e);
                    match self.stream.write(&UsbCommand::KO.as_bytes()).await {
                        Ok(_) => {
                            tracing::info!("Send KO response");
                            Err(e).context(BackendSnafu)
                        }
                        Err(e) => {
                            tracing::error!("{}", e);
                            Err(e).context(UnixStreamSnafu)
                        }
                    }
                }
            },
            UsbCommand::AngleSet { angles } => {
                match self.backend_handle.set_angles(*angles).await {
                    Ok(_) => {
                        tracing::info!("Successfully set angles to : {:?}", &angles);
                        match self.stream.write(&UsbCommand::Ok.as_bytes()).await {
                            Ok(_) => {
                                tracing::info!("Send OK");
                                Ok(())
                            }
                            Err(e) => {
                                tracing::error!("{}", e);
                                Err(e).context(UnixStreamSnafu)
                            }
                        }
                    }
                    Err(e) => {
                        tracing::error!("{}", e);
                        Err(e).context(BackendSnafu)
                    }
                }
            }
            UsbCommand::KO => {
                tracing::error!("Message not expected !");
                let err = Err(errors::Error::Unexpected {
                    reason: "Unexpected message received".to_string(),
                });
                err.context(IpcReaderSnafu)
            }
            UsbCommand::SetRole {
                number_of_parties,
                position,
            } => {
                match self
                    .backend_handle
                    .set_role(*number_of_parties, *position)
                    .await
                {
                    Ok(_) => {
                        tracing::info!(
                            "Successfully set role with nb_of_parties {} and position {}",
                            number_of_parties,
                            position
                        );
                        match self.stream.write(&UsbCommand::Ok.as_bytes()).await {
                            Ok(_) => {
                                tracing::info!("Send OK");
                                Ok(())
                            }
                            Err(e) => {
                                tracing::error!("{}", e);
                                Err(e).context(UnixStreamSnafu)
                            }
                        }
                    }
                    Err(e) => {
                        tracing::error!("{}", e);
                        Err(e).context(BackendSnafu)
                    }
                }
            }
        }
    }

    pub async fn new(unix_stream: UnixStream, backend_handle: BackendHandle<G>) -> Self {
        IPCReader {
            backend_handle,
            stream: unix_stream,
        }
    }

    pub async fn start(mut self) -> Result<(), errors::Error> {
        loop {
            let usb_cmd = match self.read_cmd().await {
                Ok(c) => c,
                Err(e) => return Err(e),
            };
            match self.process_cmd(&usb_cmd).await {
                Ok(_) => {
                    tracing::info!("Processing of {:?}: Success", &usb_cmd);
                }
                Err(_e) => {
                    tracing::error!("Processing of {:?}: Failure", &usb_cmd);
                }
            }
        }
    }
}
