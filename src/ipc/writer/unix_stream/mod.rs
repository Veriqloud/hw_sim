use std::sync::Arc;

use tokio::{io::BufWriter, net::UnixStream};

use super::{errors::IOSnafu, *};
use snafu::ResultExt;

#[derive(Clone)]
pub struct StreamWriter {
    pub writer: Arc<BufWriter<UnixStream>>,
}

impl StreamWriter {
    pub fn new(writer: Arc<BufWriter<UnixStream>>) -> Self {
        Self { writer }
    }
}

#[async_trait]
impl Writer for StreamWriter {
    async fn insert_data(&self, data: Vec<u8>) -> Result<usize, Error> {
        Ok(self.writer.get_ref().try_write(&data).context(IOSnafu)?)
    }
}
