use std::fs::File;

use super::writer::{actor::FifoWriterLease, errors::Error};

/// Owns all FIFO descriptors used by one hardware session.
pub struct FifoConnection {
    // Fields are dropped in declaration order: release the reader before the writers.
    gc_read_file: File,
    writers: FifoWriterLease,
}

impl FifoConnection {
    pub fn new(gc_read_file: File, writers: FifoWriterLease) -> Self {
        Self {
            gc_read_file,
            writers,
        }
    }

    pub fn gc_reader(&mut self) -> &mut File {
        &mut self.gc_read_file
    }

    pub fn write_gcr_batch(&self, data: Vec<[u8; 8]>) -> Result<(), Error> {
        self.writers.write_gcr_batch(data)
    }

    pub fn write_angles_batch(&self, data: Vec<u8>) -> Result<(), Error> {
        self.writers.write_angles_batch(data)
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
        let writers = writer_handle
            .clone()
            .attach_writers(
                fs::File::create(base.join(format!("gcr_{suffix}"))).unwrap(),
                fs::File::create(base.join(format!("angles_{suffix}"))).unwrap(),
            )
            .unwrap();
        FifoConnection::new(
            fs::File::create(base.join(format!("gc_read_{suffix}"))).unwrap(),
            writers,
        )
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
