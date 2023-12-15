pub mod actor;
pub mod errors;
pub mod protocols;
pub mod role;
pub mod simulation;

use libhardware::Backend;
use serde::{Deserialize, Serialize};

pub(crate) static ANGLE_PATH: &str = "./angles";

#[derive(Serialize, Deserialize)]
pub(crate) struct Angles {
    pub(crate) angles: Vec<u8>,
}

pub trait BytesGenerator: Backend + Send + Sync + Clone + 'static {}
impl<T> BytesGenerator for T where T: Backend + Send + Sync + Clone + 'static {}
