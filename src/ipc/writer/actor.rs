use snafu::ResultExt;
use tokio::{
    io::AsyncWriteExt,
    net::UnixStream,
    sync::{mpsc, oneshot},
    time::sleep,
};

use crate::errors::IOSnafu;

use super::super::super::backend::actor::ActorHandle as SimulatorHandle;
use super::errors::{ActorDiedSnafu, Error};

pub struct IPCWriterActor {
    receiver: mpsc::Receiver<WriterMessage>,
    angles_stream: UnixStream,
    click_results_stream: UnixStream,
    writing_active: bool,
    simulator_handle: SimulatorHandle,
}

impl IPCWriterActor {
    pub fn new(
        angles_stream: UnixStream,
        click_results_stream: UnixStream,
        receiver: mpsc::Receiver<WriterMessage>,
        simulator_handle: SimulatorHandle,
    ) -> Self {
        IPCWriterActor {
            receiver,
            angles_stream,
            click_results_stream,
            writing_active: false,
            simulator_handle,
        }
    }

    async fn handle_message(&mut self, msg: WriterMessage) {
        match msg {
            WriterMessage::Start => {
                self.writing_active = true;
                self.write_loop().await;
            }
            WriterMessage::Stop => {
                self.writing_active = false;
            }
        }
    }

    async fn write_loop(&mut self) {
        while self.writing_active {
            match self.simulator_handle.read_angles().await {
                Ok(data) => {
                    if let Err(e) = self.process_and_write_data(data).await {
                        tracing::error!("Failed to process and write data: {:?}", e);
                    }
                }
                Err(e) => {
                    tracing::error!("Failed to generate bytes: {:?}", e);
                }
            }
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
        self.angles_stream.write_all(&angles_data).await.unwrap();
        // .context(IOSnafu)?;

        self.click_results_stream
            .write_all(&click_results_data)
            .await
            .unwrap();
        // .context(IOSnafu)?;

        Ok(())
    }
}

pub enum WriterMessage {
    Start,
    Stop,
}

pub async fn run_writer_actor(mut actor: IPCWriterActor) {
    while let Some(msg) = actor.receiver.recv().await {
        actor.handle_message(msg).await;
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
