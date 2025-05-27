use std::fmt::Debug;
use tokio::fs::File;
use tokio::io::AsyncWriteExt;

use tokio::sync::{mpsc, Mutex, OnceCell}; // Removed oneshot components for stop_chan

use super::super::super::backend::actor::ActorHandle as SimulatorHandle;
use super::errors::Error;

// Static OnceCell for file handles, managed by the actor.
// CLICK_RESULTS is removed as it's part of GCR stream now.
static GC_WRITE_FILE: OnceCell<Mutex<File>> = OnceCell::const_new();
static ANGLES_FILE: OnceCell<Mutex<File>> = OnceCell::const_new();

pub struct IPCWriterActor {
    receiver: mpsc::Receiver<WriterMessage>,
    // simulator_handle is no longer needed here as the writer only writes what it's told.
    // stop_chan is removed; the actor stops when its message channel closes or by a specific message if needed.
}

impl IPCWriterActor {
    pub fn new(
        gc_write_file: File, // For GCR data (GC + result bit)
        angles_file: File,   // For angles data
        receiver: mpsc::Receiver<WriterMessage>,
        _simulator_handle: SimulatorHandle, // Kept for signature compatibility, but not used
    ) -> Self {
        // Attempt to set the file handles. If already set (e.g. actor restarted without process restart),
        // this might panic. Consider using get_or_init for robustness if actor can be re-created.
        GC_WRITE_FILE
            .set(Mutex::new(gc_write_file))
            .expect("GC_WRITE_FILE static OnceCell already set");
        ANGLES_FILE
            .set(Mutex::new(angles_file))
            .expect("ANGLES_FILE static OnceCell already set");

        IPCWriterActor { receiver }
    }

    async fn handle_message(&mut self, msg: WriterMessage) -> Result<(), Error> {
        match msg {
            WriterMessage::WriteGcrBatch(gcr_data_batch) => {
                tracing::info!(
                    "WriterActor: Received WriteGcrBatch ({} items).",
                    gcr_data_batch.len()
                );
                if let Some(file_mutex) = GC_WRITE_FILE.get() {
                    let mut file_guard = file_mutex.lock().await;
                    for gcr_item in gcr_data_batch {
                        file_guard.write_all(&gcr_item).await.map_err(|e| {
                            tracing::error!("Failed to write GCR item: {:?}", e);
                            Error::Channel {
                                e: format!("Failed to write GCR item to FIFO: {}", e),
                            }
                        })?;
                    }
                    file_guard.flush().await.map_err(|e| { // Ensure data is sent
                        tracing::error!("Failed to flush GCR FIFO: {:?}", e);
                        Error::Channel {
                             e: format!("Failed to flush GCR FIFO: {}", e),
                        }
                    })?;
                    tracing::info!("WriterActor: Successfully wrote GCR batch.");
                } else {
                    tracing::error!("WriterActor: GC_WRITE_FILE not initialized.");
                    return Err(Error::Channel {
                        e: "GC_WRITE_FILE not initialized".to_string(),
                    });
                }
                Ok(())
            }
            WriterMessage::WriteAnglesBatch(angles_batch) => {
                tracing::info!(
                    "WriterActor: Received WriteAnglesBatch ({} bytes).",
                    angles_batch.len()
                );
                if let Some(file_mutex) = ANGLES_FILE.get() {
                    let mut file_guard = file_mutex.lock().await;
                    file_guard.write_all(&angles_batch).await.map_err(|e| {
                        tracing::error!("Failed to write angles batch: {:?}", e);
                        Error::Channel {
                            e: format!("Failed to write angles batch to FIFO: {}", e),
                        }
                    })?;
                    file_guard.flush().await.map_err(|e| { // Ensure data is sent
                        tracing::error!("Failed to flush angles FIFO: {:?}", e);
                        Error::Channel {
                             e: format!("Failed to flush angles FIFO: {}", e),
                        }
                    })?;
                    tracing::info!("WriterActor: Successfully wrote angles batch.");
                } else {
                    tracing::error!("WriterActor: ANGLES_FILE not initialized.");
                    return Err(Error::Channel {
                        e: "ANGLES_FILE not initialized".to_string(),
                    });
                }
                Ok(())
            }
            WriterMessage::Stop => {
                // The writer actor itself doesn't have a loop to stop other than processing messages.
                // If the sender (IPCReader) stops sending messages, this actor's loop will end.
                // This message can be used for graceful shutdown if the actor had internal tasks.
                tracing::info!("WriterActor: Received Stop message. No active loops to stop in writer itself. Will stop processing further messages if channel closes.");
                // To explicitly stop the actor from processing more messages, we could close its own receiver,
                // but typically the owner (IPCReader) dropping its sender handle achieves this.
                Ok(())
            }
        }
    }
}

// Message enum for the writer actor
#[derive(Debug)]
pub enum WriterMessage {
    WriteGcrBatch(Vec<[u8; 8]>), // Batch of GCR data (Global Counter + Result bit)
    WriteAnglesBatch(Vec<u8>),   // Batch of Angle values
    Stop,                        // Command to stop (if needed for internal loops, currently informational)
}

pub async fn run_writer_actor(mut actor: IPCWriterActor) {
    tracing::info!("IPCWriterActor running.");
    while let Some(msg) = actor.receiver.recv().await {
        tracing::debug!("IPCWriterActor: Received message: {:?}", msg);
        if let Err(e) = actor.handle_message(msg).await {
            tracing::error!("IPCWriterActor: Failed to handle message: {:?}", e);
            // Depending on the error, might want to break or continue
        }
    }
    tracing::info!("IPCWriterActor finished.");
}

#[derive(Debug, Clone)]
pub struct IPCWriterActorHandle {
    sender: mpsc::Sender<WriterMessage>,
}

impl IPCWriterActorHandle {
    pub fn new(
        gc_write_file: File,
        angles_file: File,
        simulator_handle: SimulatorHandle, // Kept for signature, not directly used by new writer
    ) -> Self {
        let (sender, receiver) = mpsc::channel(8); // Channel for messages to the actor
        let actor = IPCWriterActor::new(
            gc_write_file,
            angles_file,
            receiver,
            simulator_handle,
        );
        tokio::spawn(run_writer_actor(actor)); // Spawn the actor task
        Self { sender }
    }

    pub async fn write_gcr_batch(&self, gcr_data: Vec<[u8; 8]>) -> Result<(), Error> {
        let message = WriterMessage::WriteGcrBatch(gcr_data);
        self.sender.send(message).await.map_err(|e| {
            tracing::error!("Failed to send WriteGcrBatch to IPCWriterActor: {}", e);
            Error::Channel {
                e: format!("Send GCR batch failed: {}", e),
            }
        })
    }

    pub async fn write_angles_batch(&self, angles_data: Vec<u8>) -> Result<(), Error> {
        let message = WriterMessage::WriteAnglesBatch(angles_data);
        self.sender.send(message).await.map_err(|e| {
            tracing::error!("Failed to send WriteAnglesBatch to IPCWriterActor: {}", e);
            Error::Channel {
                e: format!("Send angles batch failed: {}", e),
            }
        })
    }

    // Optional: A stop message if explicit cleanup or signaling is needed in the writer actor.
    // For now, the writer stops when its command channel is closed by the reader.
    pub async fn stop(&self) -> Result<(), Error> {
        let message = WriterMessage::Stop;
        self.sender.send(message).await.map_err(|e| {
            tracing::error!("Failed to send Stop to IPCWriterActor: {}", e);
            Error::Channel {
                e: format!("Send Stop failed: {}", e),
            }
        })
    }
}
