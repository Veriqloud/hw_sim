use super::errors::Error;
use std::fmt::Debug;
use std::fs::File;
use std::io::Write;
use std::sync::mpsc;

pub struct IPCWriterActor {
    receiver: mpsc::Receiver<WriterMessage>,
    gcr_file: File,
    angles_file: File,
}

impl IPCWriterActor {
    pub fn new(
        gcr_file: File,    // For GCR data (GC + result bit)
        angles_file: File, // For angles data
        receiver: mpsc::Receiver<WriterMessage>,
    ) -> Self {
        IPCWriterActor {
            receiver,
            gcr_file,
            angles_file,
        }
    }

    fn handle_message(&mut self, msg: WriterMessage) -> Result<(), Error> {
        match msg {
            WriterMessage::WriteGcrBatch(gcr_data_batch) => {
                tracing::info!(
                    "WriterActor: Received WriteGcrBatch ({} items).",
                    gcr_data_batch.len()
                );

                // --- Batched Write Optimization (Safe Version) ---
                // Flatten the Vec<[u8; 8]> into a single Vec<u8> for one write call.
                // This is a safe operation that copies the data into a contiguous buffer.
                let buffer: Vec<u8> = gcr_data_batch.into_iter().flatten().collect();
                self.gcr_file
                    .write_all(&buffer)
                    .map_err(|e| Error::Channel {
                        e: format!("Failed to write GCR batch to FIFO: {}", e),
                    })?;

                self.gcr_file.flush().map_err(|e| {
                    // Ensure data is sent
                    tracing::error!("Failed to flush GCR FIFO: {:?}", e);
                    Error::Channel {
                        e: format!("Failed to flush GCR FIFO: {}", e),
                    }
                })?;
                tracing::info!("WriterActor: Successfully wrote GCR batch.");
                Ok(())
            }
            WriterMessage::WriteAnglesBatch(angles_batch) => {
                tracing::info!(
                    "WriterActor: Received WriteAnglesBatch ({} raw bytes), packing them.",
                    angles_batch.len()
                );

                // Pack angle indices. The raw data from the simulator has the 2-bit index
                // in bits 1 and 2 of each byte. We take two such bytes and pack their
                // indices into a single byte.
                // The format is 0b00xx00yy, where yy is the index from the first byte
                // and xx is the index from the second byte.
                let packed_angles: Vec<u8> = angles_batch
                    .chunks_exact(2)
                    .map(|chunk| {
                        let index1 = chunk[0] & 0b0000_0011;
                        let index2 = chunk[1] & 0b0000_0011;
                        // Combine into a single byte: 0b00xx00yy
                        // index1 goes into the lower bits (yy)
                        // index2 goes into bits 4 and 5 (xx)
                        index1 | (index2 << 4)
                    })
                    .collect();

                if angles_batch.len() % 2 != 0 {
                    tracing::warn!(
                        "WriterActor: Odd number of angle bytes received ({}). The last byte will be ignored.",
                        angles_batch.len()
                    );
                }

                tracing::info!(
                    "WriterActor: Writing {} packed angle bytes.",
                    packed_angles.len()
                );
                self.angles_file.write_all(&packed_angles).map_err(|e| {
                    tracing::error!("Failed to write packed angles batch: {:?}", e);
                    Error::Channel {
                        e: format!("Failed to write packed angles batch to FIFO: {}", e),
                    }
                })?;
                self.angles_file.flush().map_err(|e| {
                    // Ensure data is sent
                    tracing::error!("Failed to flush packed angles FIFO: {:?}", e);
                    Error::Channel {
                        e: format!("Failed to flush packed angles FIFO: {}", e),
                    }
                })?;
                tracing::info!("WriterActor: Successfully wrote packed angles batch.");
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
    Stop, // Command to stop (if needed for internal loops, currently informational)
}

pub fn run_writer_actor(mut actor: IPCWriterActor) {
    tracing::info!("IPCWriterActor running.");
    while let Ok(msg) = actor.receiver.recv() {
        tracing::debug!("IPCWriterActor: Received message: {:?}", msg);
        if let Err(e) = actor.handle_message(msg) {
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
        gcr_file: File, // Renamed parameter to match usage
        angles_file: File,
    ) -> Self {
        let (sender, receiver) = mpsc::channel();
        let actor = IPCWriterActor::new(gcr_file, angles_file, receiver);
        std::thread::spawn(move || {
            run_writer_actor(actor);
        });

        Self { sender }
    }

    pub fn write_gcr_batch(&self, gcr_data: Vec<[u8; 8]>) -> Result<(), Error> {
        let message = WriterMessage::WriteGcrBatch(gcr_data);
        self.sender.send(message).map_err(|e| {
            tracing::error!("Failed to send WriteGcrBatch to IPCWriterActor: {}", e);
            Error::Channel {
                e: format!("Send GCR batch failed: {}", e),
            }
        })
    }

    pub fn write_angles_batch(&self, angles_data: Vec<u8>) -> Result<(), Error> {
        let message = WriterMessage::WriteAnglesBatch(angles_data);
        self.sender.send(message).map_err(|e| {
            tracing::error!("Failed to send WriteAnglesBatch to IPCWriterActor: {}", e);
            Error::Channel {
                e: format!("Send angles batch failed: {}", e),
            }
        })
    }

    // Optional: A stop message if explicit cleanup or signaling is needed in the writer actor.
    // For now, the writer stops when its command channel is closed by the reader.
    pub fn stop(&self) -> Result<(), Error> {
        let message = WriterMessage::Stop;
        self.sender.send(message).map_err(|e| {
            tracing::error!("Failed to send Stop to IPCWriterActor: {}", e);
            Error::Channel {
                e: format!("Send Stop failed: {}", e),
            }
        })
    }
}
