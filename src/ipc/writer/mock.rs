use super::*;
pub struct MockInsert {}

#[async_trait]
impl Writer for MockInsert {
    async fn insert_data(&self, _data: Vec<u8>) -> Result<(), Error> {
        Ok(())
    }

    async fn start(&mut self) -> Result<(), Error> {
        Ok(())
    }
}
