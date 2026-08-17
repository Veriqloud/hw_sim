pub mod fifo_connection;
pub mod writer;

use serde::Deserialize;
use serde::Serialize;

#[derive(Serialize, Deserialize, Debug)]
pub enum Command {
    Start = 0x27, // start generating bits and fill continuously the fifos
    Stop = 0x26,  // stop the generating bits and filling the fifos
}
