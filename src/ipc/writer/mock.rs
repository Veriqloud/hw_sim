use super::*;
pub struct MockInsert {}

#[async_trait]
impl Writer for MockInsert {
    async fn insert_keys(&self, _keys: Keys) -> Result<(), Error> {
        Ok(())
    }

    async fn start(&mut self) -> Result<(), Error> {
        Ok(())
    }
}
