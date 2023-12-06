use snafu::prelude::*;

#[derive(Debug, Snafu)]
#[snafu(visibility(pub))]
pub enum Error {
    #[snafu(display("could not connect to Unix Stream because : {}", source))]
    ActorDied {
        source: tokio::sync::oneshot::error::RecvError,
    },
    #[snafu(display("could not connect to Unix Stream because : {}", source))]
    SerdeJson { source: serde_json::Error },
    #[snafu(display("Simulator error : {}", source))]
    Simulation {
        source: crate::backend::simulation::errors::Error,
    },
    #[snafu(display("Hardware error : {}", source))]
    Hardware {
        source: libhardware::errors::HardwareError,
    },
    #[snafu(display("IO Error : {}", source))]
    Io { source: std::io::Error },
}
