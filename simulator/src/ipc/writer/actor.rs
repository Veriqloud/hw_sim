use super::errors::Error;
use std::fmt::Debug;
use std::fs::File;
use std::io::Write;
use std::sync::mpsc;

pub struct IPCWriterActor {
    receiver: mpsc::Receiver<WriterMessage>,
    gcr_file: Option<File>,
    angles_file: Option<File>,
}

impl IPCWriterActor {
    pub fn new(receiver: mpsc::Receiver<WriterMessage>) -> Self {
        IPCWriterActor {
            receiver,
            gcr_file: None,
            angles_file: None,
        }
    }

    fn write_gcr_batch(&mut self, gcr_data_batch: Vec<[u8; 8]>) -> Result<(), Error> {
        tracing::info!(
            "WriterActor: Received WriteGcrBatch ({} items).",
            gcr_data_batch.len()
        );

        let gcr_file = self
            .gcr_file
            .as_mut()
            .ok_or(Error::WriterUnavailable { writer: "GCR" })?;

        // Flatten the Vec<[u8; 8]> into a single Vec<u8> for one write call.
        let buffer: Vec<u8> = gcr_data_batch.into_iter().flatten().collect();
        gcr_file.write_all(&buffer).map_err(|e| Error::Channel {
            e: format!("Failed to write GCR batch to FIFO: {}", e),
        })?;
        gcr_file.flush().map_err(|e| Error::Channel {
            e: format!("Failed to flush GCR FIFO: {}", e),
        })?;
        tracing::info!("WriterActor: Successfully wrote GCR batch.");
        Ok(())
    }

    fn write_angles_batch(&mut self, angles_batch: Vec<u8>) -> Result<(), Error> {
        tracing::info!(
            "WriterActor: Received WriteAnglesBatch ({} pre-packed bytes).",
            angles_batch.len()
        );

        let angles_file = self
            .angles_file
            .as_mut()
            .ok_or(Error::WriterUnavailable { writer: "angles" })?;
        angles_file.write_all(&angles_batch).map_err(|e| {
            tracing::error!("Failed to write angles batch: {:?}", e);
            Error::Channel {
                e: format!("Failed to write angles batch to FIFO: {}", e),
            }
        })?;
        angles_file.flush().map_err(|e| Error::Channel {
            e: format!("Failed to flush angles FIFO: {}", e),
        })?;
        tracing::info!("WriterActor: Successfully wrote angles batch.");
        Ok(())
    }

    fn attach_writers(&mut self, gcr_file: File, angles_file: File) -> Result<(), Error> {
        if self.gcr_file.is_some() || self.angles_file.is_some() {
            return Err(Error::WritersAlreadyOpen);
        }

        self.gcr_file = Some(gcr_file);
        self.angles_file = Some(angles_file);
        tracing::info!("WriterActor: IPC writer files attached.");
        Ok(())
    }

    fn close_writers(&mut self) -> Result<(), Error> {
        drop(self.gcr_file.take());
        drop(self.angles_file.take());
        tracing::info!("WriterActor: IPC writer files closed.");
        Ok(())
    }

    fn handle_message(&mut self, msg: WriterMessage) {
        match msg {
            WriterMessage::WriteGcrBatch(data) => {
                if let Err(e) = self.write_gcr_batch(data) {
                    tracing::error!("IPCWriterActor: Failed to write GCR batch: {:?}", e);
                }
            }
            WriterMessage::WriteAnglesBatch(data) => {
                if let Err(e) = self.write_angles_batch(data) {
                    tracing::error!("IPCWriterActor: Failed to write angles batch: {:?}", e);
                }
            }
            WriterMessage::AttachWriters {
                gcr_file,
                angles_file,
                reply_to,
            } => {
                send_reply(reply_to, self.attach_writers(gcr_file, angles_file));
            }
            WriterMessage::CloseWriters { reply_to } => {
                send_reply(reply_to, self.close_writers());
            }
        }
    }
}

fn send_reply(reply_to: mpsc::Sender<Result<(), Error>>, result: Result<(), Error>) {
    if reply_to.send(result).is_err() {
        tracing::warn!("IPCWriterActor: Requester dropped before receiving its reply.");
    }
}

#[derive(Debug)]
pub enum WriterMessage {
    WriteGcrBatch(Vec<[u8; 8]>),
    WriteAnglesBatch(Vec<u8>),
    AttachWriters {
        gcr_file: File,
        angles_file: File,
        reply_to: mpsc::Sender<Result<(), Error>>,
    },
    CloseWriters {
        reply_to: mpsc::Sender<Result<(), Error>>,
    },
}

pub fn run_writer_actor(mut actor: IPCWriterActor) {
    tracing::info!("IPCWriterActor running.");
    while let Ok(msg) = actor.receiver.recv() {
        tracing::debug!("IPCWriterActor: Received message: {:?}", msg);
        actor.handle_message(msg);
    }
    tracing::info!("IPCWriterActor finished.");
}

#[derive(Debug, Clone)]
pub struct IPCWriterActorHandle {
    sender: mpsc::Sender<WriterMessage>,
}

pub struct FifoWriterLease {
    writer_handle: IPCWriterActorHandle,
}

impl FifoWriterLease {
    pub fn write_gcr_batch(&self, data: Vec<[u8; 8]>) -> Result<(), Error> {
        self.writer_handle.write_gcr_batch(data)
    }

    pub fn write_angles_batch(&self, data: Vec<u8>) -> Result<(), Error> {
        self.writer_handle.write_angles_batch(data)
    }
}

impl Drop for FifoWriterLease {
    fn drop(&mut self) {
        if let Err(error) = self.writer_handle.close_writers() {
            tracing::error!("Failed to close FIFO writers: {}", error);
        }
    }
}

impl IPCWriterActorHandle {
    pub fn new() -> Self {
        let (sender, receiver) = mpsc::channel();
        let actor = IPCWriterActor::new(receiver);
        std::thread::spawn(move || {
            run_writer_actor(actor);
        });

        Self { sender }
    }

    fn send_command_and_wait(
        &self,
        make_message: impl FnOnce(mpsc::Sender<Result<(), Error>>) -> WriterMessage,
    ) -> Result<(), Error> {
        let (reply_to, reply_from) = mpsc::channel();
        self.sender
            .send(make_message(reply_to))
            .map_err(|e| Error::Channel {
                e: format!("Failed to send message to IPCWriterActor: {}", e),
            })?;
        reply_from
            .recv()
            .map_err(|source| Error::ActorDied { source })?
    }

    fn write_gcr_batch(&self, data: Vec<[u8; 8]>) -> Result<(), Error> {
        self.sender
            .send(WriterMessage::WriteGcrBatch(data))
            .map_err(|e| Error::Channel {
                e: format!("Failed to send GCR batch to IPCWriterActor: {}", e),
            })
    }

    fn write_angles_batch(&self, data: Vec<u8>) -> Result<(), Error> {
        self.sender
            .send(WriterMessage::WriteAnglesBatch(data))
            .map_err(|e| Error::Channel {
                e: format!("Failed to send angles batch to IPCWriterActor: {}", e),
            })
    }

    pub fn attach_writers(
        self,
        gcr_file: File,
        angles_file: File,
    ) -> Result<FifoWriterLease, Error> {
        self.send_command_and_wait(|reply_to| WriterMessage::AttachWriters {
            gcr_file,
            angles_file,
            reply_to,
        })?;
        Ok(FifoWriterLease {
            writer_handle: self,
        })
    }

    fn close_writers(&self) -> Result<(), Error> {
        self.send_command_and_wait(|reply_to| WriterMessage::CloseWriters { reply_to })
    }
}

#[cfg(test)]
mod tests {
    use super::IPCWriterActorHandle;
    use std::fs;
    use uuid::Uuid;

    #[test]
    fn cloned_handles_survive_writer_close_and_replacement() {
        let base = std::env::temp_dir().join(format!("hw_sim_writer_{}", Uuid::new_v4()));
        fs::create_dir_all(&base).unwrap();
        let handle = IPCWriterActorHandle::new();
        let cloned_handle = handle.clone();

        let first_gcr_path = base.join("gcr_1");
        let first_angles_path = base.join("angles_1");
        let first_lease = handle
            .clone()
            .attach_writers(
                fs::File::create(&first_gcr_path).unwrap(),
                fs::File::create(&first_angles_path).unwrap(),
            )
            .unwrap();
        first_lease.write_angles_batch(vec![1, 2, 3, 4]).unwrap();
        drop(first_lease);

        assert_eq!(fs::read(&first_angles_path).unwrap(), vec![1, 2, 3, 4]);
        let second_gcr_path = base.join("gcr_2");
        let second_angles_path = base.join("angles_2");
        let second_lease = cloned_handle
            .attach_writers(
                fs::File::create(&second_gcr_path).unwrap(),
                fs::File::create(&second_angles_path).unwrap(),
            )
            .unwrap();
        second_lease.write_angles_batch(vec![6, 7]).unwrap();
        drop(second_lease);

        assert_eq!(fs::read(&second_angles_path).unwrap(), vec![6, 7]);
        fs::remove_dir_all(base).unwrap();
    }
}
