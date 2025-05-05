use snafu::ResultExt;
use std::sync::{Arc, OnceLock};
use std::time::Duration;
use tokio::io::AsyncWriteExt;

use tokio::net::UnixStream;
use tokio::sync::oneshot::Receiver;
use tokio::sync::{mpsc, oneshot, Mutex, OnceCell};

use super::errors::{Error, IOSnafu};
use super::{super::super::backend::actor::ActorHandle as SimulatorHandle, errors::BackendSnafu};

static ANGLES_STREAM: OnceCell<Mutex<UnixStream>> = OnceCell::const_new();

pub struct IPCWriterActor {
    receiver: mpsc::Receiver<WriterMessage>,
    click_results_stream: UnixStream,
    simulator_handle: SimulatorHandle,
    stop_chan: Option<oneshot::Sender<()>>,
}

impl IPCWriterActor {
    pub fn new(
        angles_stream: UnixStream,
        click_results_stream: UnixStream,
        receiver: mpsc::Receiver<WriterMessage>,
        simulator_handle: SimulatorHandle,
    ) -> Self {
        ANGLES_STREAM.set(Mutex::new(angles_stream));

        IPCWriterActor {
            receiver,
            click_results_stream,
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
                let (send, mut recv) = oneshot::channel();
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
                        Some(chan) => {
                            chan.send(());
                        }
                        None => todo!(),
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
                _ = tokio::time::sleep(tokio::time::Duration::from_millis(10))=>{

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
                        .write(&angles_data);
                }
                Err(e) => {
                    tracing::error!("Failed to generate bytes: {:?}", e);
                    break;
                }
            }

            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    }

    async fn process_and_write_data(data: [u8; 1024]) -> Result<(), Error> {
        self.angles_stream
            .write_all(&angles_data)
            .await
            .context(IOSnafu)?;

        self.click_results_stream
            .write_all(&click_results_data)
            .await
            .context(IOSnafu)?;

        Ok(())
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
        angles_stream: UnixStream,
        click_results_stream: UnixStream,
        simulator_handle: SimulatorHandle,
    ) -> Self {
        let (sender, receiver) = mpsc::channel(8);
        let writing_active = Arc::new(Mutex::new(false));
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
