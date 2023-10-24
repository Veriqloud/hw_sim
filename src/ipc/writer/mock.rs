use super::*;
#[derive(Clone)]
pub struct MockInsert {}

#[async_trait]
impl Writer for MockInsert {
    async fn insert_data(&self, _data: Vec<u8>) -> Result<usize, Error> {
        Ok(0)
    }
}
