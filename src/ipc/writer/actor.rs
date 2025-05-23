use std::fmt::Debug;
use tokio::fs::File;
use tokio::io::AsyncWriteExt;

use tokio::sync::oneshot::Receiver;
use tokio::sync::{mpsc, oneshot, Mutex, OnceCell};

use super::super::super::backend::actor::ActorHandle as SimulatorHandle;
use super::errors::Error;

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
                tracing::info!("Writer actor received Start message, spawning write_loop.");
                // Simulator is started and seeded by IPCReader now.
                let sim_h_cpy = self.simulator_handle.clone();
                let (send, recv) = oneshot::channel();
                self.stop_chan = Some(send);
                tokio::spawn(async move { Self::write_loop(sim_h_cpy, recv).await });
                Ok(())
            }
            WriterMessage::Stop => {
                tracing::info!("Writer actor received Stop message");
                // Simulator is stopped by IPCReader now.
                // This actor only needs to stop its own write_loop.
                {
                    let stop_chan = self.stop_chan.take();
                    match stop_chan {
                        Some(chan) => {
                            if chan.send(()).is_err() {
                                // Log if sending fails, but don't error out the actor handling
                                // as the loop might already be stopping or stopped.
                                tracing::warn!("Failed to send on stop_chan; write_loop might already be stopped.");
                            }
                        }
                        None => {
                            // If stop_chan is None, it means stop has already been called.
                            // This is not an error condition for the actor's message handling.
                            tracing::debug!("Stop channel already taken; stop process likely initiated or completed.");
                        }
                    }
                }
                tracing::info!("Writing inactive!"); // This log might be reached multiple times if Stop is processed repeatedly by reader
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
                    if data.len() % 2 != 0 {
                        tracing::error!(
                            "Received data with odd length {}, cannot process in pairs.",
                            data.len()
                        );
                        break;
                    }
                    let (angles_data, click_results_data): (Vec<u8>, Vec<u8>) = data
                        .chunks_exact(2)
                        .map(|chunk| {
                            let byte1 = chunk[0];
                            let byte2 = chunk[1];
                            let angle_byte = ((byte1 & 0b110) >> 1) | ((byte2 & 0b110) << 3);
                            let result_byte = (byte1 & 0b001) | ((byte2 & 0b001) << 4);
                            (angle_byte, result_byte)
                        })
                        .unzip();

                    if let Some(angles_stream_mutex) = ANGLES_STREAM.get() {
                        if let Err(e) = angles_stream_mutex
                            .lock()
                            .await
                            .write_all(&angles_data)
                            .await
                        {
                            tracing::error!("Failed to write angles_data: {:?}", e);
                            break; // Or handle error appropriately
                        }
                    } else {
                        tracing::error!("ANGLES_STREAM not initialized");
                        break;
                    }

                    if let Some(click_results_mutex) = CLICK_RESULTS.get() {
                        if let Err(e) = click_results_mutex
                            .lock()
                            .await
                            .write_all(&click_results_data)
                            .await
                        {
                            tracing::error!("Failed to write click_results_data: {:?}", e);
                            break; // Or handle error appropriately
                        }
                    } else {
                        tracing::error!("CLICK_RESULTS not initialized");
                        break;
                    }
                }
                Err(e) => {
                    tracing::error!("Failed to read angles (raw data) from simulator: {:?}", e);
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
