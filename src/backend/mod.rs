pub mod actor;
pub mod errors;
pub mod protocols;
pub mod role;
pub mod simulation;

use libhardware::Backend;

pub trait BytesGenerator: Backend + Send + Sync + Clone + 'static {}
impl<T> BytesGenerator for T where T: Backend + Send + Sync + Clone + 'static {}
