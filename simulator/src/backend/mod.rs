pub mod actor;
pub mod errors;
pub mod protocols;
pub mod role;
pub mod simulation;
#[cfg(test)]
pub mod tests;

use self::simulation::VqSim;

pub trait BytesGenerator: VqSim + Send + Sync + 'static {}
impl<T> BytesGenerator for T where T: VqSim + Send + Sync + 'static {}
