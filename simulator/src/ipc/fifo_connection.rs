use std::fs::File;

use super::writer::{actor::IPCWriterActorHandle, errors::Error};

/// Owns all FIFO descriptors used by one hardware session.
pub struct FifoConnection {
    gc_read_file: Option<File>,
    writer_handle: IPCWriterActorHandle,
}

impl FifoConnection {
    pub fn new(
        gc_read_file: File,
        writer_handle: IPCWriterActorHandle,
        gcr_file: File,
        angles_file: File,
    ) -> Result<Self, Error> {
        writer_handle.attach_writers(gcr_file, angles_file)?;
        Ok(Self {
            gc_read_file: Some(gc_read_file),
            writer_handle,
        })
    }

    pub fn gc_reader(&mut self) -> &mut File {
        self.gc_read_file
            .as_mut()
            .expect("FIFO reader is unavailable while the connection is alive")
    }

    pub fn write_gcr_batch(&self, data: Vec<[u8; 8]>) -> Result<(), Error> {
        self.writer_handle.write_gcr_batch(data)
    }

    pub fn write_angles_batch(&self, data: Vec<u8>) -> Result<(), Error> {
        self.writer_handle.write_angles_batch(data)
    }
}

impl Drop for FifoConnection {
    fn drop(&mut self) {
        drop(self.gc_read_file.take());
        if let Err(error) = self.writer_handle.close_writers() {
            tracing::error!("Failed to close FIFO writers: {}", error);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, io};

    use uuid::Uuid;

    use super::FifoConnection;
    use crate::ipc::writer::actor::IPCWriterActorHandle;

    fn open_connection(
        base: &std::path::Path,
        suffix: &str,
        writer_handle: &IPCWriterActorHandle,
    ) -> FifoConnection {
        FifoConnection::new(
            fs::File::create(base.join(format!("gc_read_{suffix}"))).unwrap(),
            writer_handle.clone(),
            fs::File::create(base.join(format!("gcr_{suffix}"))).unwrap(),
            fs::File::create(base.join(format!("angles_{suffix}"))).unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn error_return_drops_connection_and_closes_writers() {
        let base = std::env::temp_dir().join(format!("hw_sim_fifo_{}", Uuid::new_v4()));
        fs::create_dir_all(&base).unwrap();
        let writer_handle = IPCWriterActorHandle::new();

        let result = (|| -> Result<(), io::Error> {
            let connection = open_connection(&base, "first", &writer_handle);
            connection.write_angles_batch(vec![1, 2, 3]).unwrap();
            Err(io::Error::other("session failed"))
        })();
        assert!(result.is_err());

        let connection = open_connection(&base, "second", &writer_handle);
        connection.write_angles_batch(vec![4, 5]).unwrap();
        drop(connection);

        assert_eq!(fs::read(base.join("angles_first")).unwrap(), vec![1, 2, 3]);
        assert_eq!(fs::read(base.join("angles_second")).unwrap(), vec![4, 5]);
        fs::remove_dir_all(base).unwrap();
    }
}
