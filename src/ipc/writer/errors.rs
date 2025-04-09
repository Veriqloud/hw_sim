use snafu::prelude::*;

#[derive(Debug, Snafu)]
#[snafu(visibility(pub))]
pub enum Error {
    #[snafu(display("could not connect to Unix Stream because : {}", source))]
    ActorDied {
        source: tokio::sync::oneshot::error::RecvError,
    },
    #[snafu(display("Could not write to Unix Stream because : {}", source))]
    IO { source: std::io::Error },
}
