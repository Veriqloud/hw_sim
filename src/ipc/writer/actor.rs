use std::sync::{Arc, Mutex};
use std::time::Duration;

use snafu::ResultExt;
use tokio::io::AsyncWriteExt;
use tokio::net::UnixStream;
use tokio::sync::mpsc;
use tokio::time::sleep;

use super::errors::{ActorDiedSnafu, Error, IOSnafu};
use super::{super::super::backend::actor::ActorHandle as SimulatorHandle, errors::BackendSnafu};

pub struct IPCWriterActor {
    receiver: mpsc::Receiver<WriterMessage>,
    angles_stream: UnixStream,
    click_results_stream: UnixStream,
    writing_active: Arc<Mutex<bool>>,
    simulator_handle: SimulatorHandle,
}

impl IPCWriterActor {
    pub fn new(
        angles_stream: UnixStream,
        click_results_stream: UnixStream,
        receiver: mpsc::Receiver<WriterMessage>,
        simulator_handle: SimulatorHandle,
        writing_active: Arc<Mutex<bool>>,
    ) -> Self {
        IPCWriterActor {
            receiver,
            angles_stream,
            click_results_stream,
            writing_active,
            simulator_handle,
        }
    }

    async fn handle_message(&mut self, msg: WriterMessage) -> Result<(), Error> {
        match msg {
            WriterMessage::Start => {
                tracing::info!("Writer actor received Start message");
                self.simulator_handle.start().await.context(BackendSnafu)?;
                *self.writing_active.lock().unwrap() = true;
                self.write_loop().await;
                Ok(())
            }
            WriterMessage::Stop => {
                tracing::info!("Writer actor received Stop message");
                self.simulator_handle.stop().await.context(BackendSnafu)?;
                *self.writing_active.lock().unwrap() = false;
                tracing::info!("Writing inactive!");
                Ok(())
            }
        }
    }

    async fn write_loop(&mut self) {
        while *self.writing_active.lock().unwrap() {
            match self.simulator_handle.read_angles().await {
                Ok(data) => {
                    if let Err(e) = self.process_and_write_data(data).await {
                        tracing::error!("Failed to process and write data: {:?}", e);
                    }
                }
                Err(e) => {
                    tracing::error!("Failed to generate bytes: {:?}", e);
                    break;
                }
            }

            if !*self.writing_active.lock().unwrap() {
                tracing::info!("Stop signal received, exiting write_loop");
                break;
            }

            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    }

    async fn process_and_write_data(&mut self, data: [u8; 1024]) -> Result<(), Error> {
        let (angles_data, click_results_data): (Vec<u8>, Vec<u8>) = data
            .iter()
            .map(|&byte| {
                let basis = (byte & 0b01000000) != 0;
                let measurement = (byte & 0b00000001) != 0;
                ((basis as u8), (measurement as u8))
            })
            .unzip();
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
            writing_active.clone(),
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
