use sim_lib::errors::SimulationError;
use snafu::prelude::*;

#[derive(Debug, Snafu)]
#[snafu(visibility(pub))]
pub enum Error {
    #[snafu(display("could not connect to Unix Stream because : {}", source))]
    ActorDied { source: std::sync::mpsc::RecvError },
    #[snafu(display("SerdeJson error because : {}", source))]
    SerdeJson { source: serde_json::Error },
    #[snafu(display("Simulator error : {}", source))]
    Simulation { source: SimulationError },
    #[snafu(display("IO Error : {}", source))]
    Io { source: std::io::Error },
    #[snafu(display("Actor channel send error : {}", e))]
    ActorSend { e: String },
}
