use snafu::ResultExt;
use std::fmt::Debug;
use tokio::fs::File;
use tokio::io::AsyncWriteExt;

use tokio::sync::oneshot::Receiver;
use tokio::sync::{mpsc, oneshot, Mutex, OnceCell};

use super::errors::Error;
use super::{super::super::backend::actor::ActorHandle as SimulatorHandle, errors::BackendSnafu};

static ANGLES_STREAM: OnceCell<Mutex<File>> = OnceCell::const_new();
static CLICK_RESULTS: OnceCell<Mutex<File>> = OnceCell::const_new();

pub struct IPCWriterActor {
    receiver: mpsc::Receiver<WriterMessage>,
    simulator_handle: SimulatorHandle,
    stop_chan: Option<oneshot::Sender<()>>,
}

impl IPCWriterActor {
    pub fn new(
        angles_stream: File,
        click_results_stream: File,
        receiver: mpsc::Receiver<WriterMessage>,
        simulator_handle: SimulatorHandle,
    ) -> Self {
        ANGLES_STREAM.set(Mutex::new(angles_stream)).unwrap();
        CLICK_RESULTS.set(Mutex::new(click_results_stream)).unwrap();

        IPCWriterActor {
            receiver,
            simulator_handle,
            stop_chan: Default::default(),
        }
    }

    async fn handle_message(&mut self, msg: WriterMessage) -> Result<(), Error> {
        match msg {
            WriterMessage::Start => {
                tracing::info!("Writer actor received Start message");
                self.simulator_handle.start().await.context(BackendSnafu)?;
                let sim_h_cpy = self.simulator_handle.clone();
                let (send, recv) = oneshot::channel();
                self.stop_chan = Some(send);
                tokio::spawn(async move { Self::write_loop(sim_h_cpy, recv).await });
                Ok(())
            }
            WriterMessage::Stop => {
                tracing::info!("Writer actor received Stop message");
                self.simulator_handle.stop().await.context(BackendSnafu)?;
                {
                    let stop_chan = self.stop_chan.take();
                    match stop_chan {
                        Some(chan) => match chan.send(()) {
                            Ok(_) => (),
                            Err(_) => {
                                return Err(Error::Channel {
                                    e: "Couldn't send through stop channel !".to_string(),
                                })
                            }
                        },
                        None => {
                            return Err(Error::Channel {
                                e: "No stop channel !".to_string(),
                            })
                        }
                    }
                }
                tracing::info!("Writing inactive!");
                Ok(())
            }
        }
    }

    async fn write_loop(simulator_handle: SimulatorHandle, mut stop_recv: Receiver<()>) {
        loop {
            tokio::select! {
                _ = &mut stop_recv =>{
                    return
                }
                _ = tokio::time::sleep(tokio::time::Duration::from_nanos(10))=>{

                }
            }

            match simulator_handle.read_angles().await {
                Ok(data) => {
                    let (angles_data, click_results_data): (Vec<u8>, Vec<u8>) = data
                        .iter()
                        .map(|&byte| {
                            let basis = (byte & 0b01000000) != 0;
                            let measurement = (byte & 0b00000001) != 0;
                            ((basis as u8), (measurement as u8))
                        })
                        .unzip();

                    // TODO: handle error
                    ANGLES_STREAM
                        .get()
                        .unwrap()
                        .lock()
                        .await
                        .write_all(&angles_data)
                        .await
                        .unwrap();
                    CLICK_RESULTS
                        .get()
                        .unwrap()
                        .lock()
                        .await
                        .write_all(&click_results_data)
                        .await
                        .unwrap();
                }
                Err(e) => {
                    tracing::error!("Failed to generate bytes: {:?}", e);
                    break;
                }
            }
        }
    }
}

#[derive(Debug)]
pub enum WriterMessage {
    Start,
    Stop,
}

pub async fn run_writer_actor(mut actor: IPCWriterActor) {
    while let Some(msg) = actor.receiver.recv().await {
        tracing::info!("Received message: {:?}", msg);
        if let Err(e) = actor.handle_message(msg).await {
            tracing::error!("Failed to handle message: {:?}", e);
        }
    }
}

#[derive(Debug, Clone)]
pub struct IPCWriterActorHandle {
    sender: mpsc::Sender<WriterMessage>,
}

impl IPCWriterActorHandle {
    pub fn new(
        angles_stream: File,
        click_results_stream: File,
        simulator_handle: SimulatorHandle,
    ) -> Self {
        let (sender, receiver) = mpsc::channel(8);
        let actor = IPCWriterActor::new(
            angles_stream,
            click_results_stream,
            receiver,
            simulator_handle,
        );
        tokio::spawn(run_writer_actor(actor));
        Self { sender }
    }

    pub async fn start(&self) -> Result<(), Error> {
        let message = WriterMessage::Start;
        let _ = self.sender.send(message).await;
        Ok(())
    }

    pub async fn stop(&self) -> Result<(), Error> {
        let message = WriterMessage::Stop;
        let _ = self.sender.send(message).await;
        Ok(())
    }
}
