pub mod actor;
pub mod errors;

use errors::Error;

use crate::simulator::Keys;
use async_trait::async_trait;

#[async_trait]
pub trait Insertor: Send + Sync + 'static {
    // Start all connections to fifos
    async fn start(&mut self) -> Result<(), Error>;
    async fn insert_keys(&self, keys: Keys) -> Result<(), Error>;
}

pub mod fifo {
    use super::*;
    pub struct MockInsert {}

    #[async_trait]
    impl Insertor for MockInsert {
        async fn insert_keys(&self, _keys: Keys) -> Result<(), Error> {
            Ok(())
        }

        async fn start(&mut self) -> Result<(), Error> {
            Ok(())
        }
    }
}
