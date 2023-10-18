use self::errors::Error;


use async_trait::async_trait;

pub mod actor;
pub mod errors;
pub mod mock;

#[async_trait]
pub trait Writer: Send + Sync + 'static {
    // Start all connections to fifos
    async fn start(&mut self) -> Result<(), Error>;
    async fn insert_data(&self, data: Vec<u8>) -> Result<(), Error>;
}
