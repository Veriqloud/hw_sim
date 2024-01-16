pub mod actor;
pub mod errors;
pub mod protocols;
pub mod role;
pub mod simulation;


use serde::{Deserialize, Serialize};

use self::simulation::VqSim;

pub(crate) static ANGLE_PATH: &str = "./angles.json";

#[derive(Serialize, Deserialize)]
pub(crate) struct Angles {
    pub(crate) angles: Vec<u8>,
}

pub trait BytesGenerator: VqSim + Send + Sync + Clone + 'static {}
impl<T> BytesGenerator for T where T: VqSim + Send + Sync + Clone + 'static {}
