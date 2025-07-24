use snafu::prelude::*;

#[derive(Debug, Snafu)]
#[snafu(visibility(pub))]
pub enum Error {
    #[snafu(display("Stop channel error because : {}", e))]
    Channel { e: String },
    #[snafu(display("could not connect to Unix Stream because : {}", source))]
    ActorDied {
        source: tokio::sync::oneshot::error::RecvError,
    },
    #[snafu(display("Backend Error : {}", source))]
    Backend {
        source: crate::backend::errors::Error,
    },
    #[snafu(display("Could not write to Unix Stream because : {}", source))]
    IO { source: std::io::Error },
}
