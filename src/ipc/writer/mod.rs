use self::errors::Error;

use async_trait::async_trait;

pub mod actor;
pub mod errors;
pub mod mock;
pub mod unix_stream;

#[async_trait]
pub trait Writer: Send + Sync + 'static {
    async fn insert_data(&self, data: Vec<u8>) -> Result<usize, Error>;
}
