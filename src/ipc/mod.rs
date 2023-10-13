pub mod errors;
pub mod reader;
pub mod writer;

use serde::Deserialize;
use serde::Serialize;

#[derive(Serialize, Deserialize, Debug)]
pub struct KeygenRequest {
    pub size: usize,
    pub owner: String,
}
