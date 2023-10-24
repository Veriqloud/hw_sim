use snafu::prelude::*;

#[derive(Debug, Snafu)]
#[snafu(visibility(pub))]
pub enum Error {
    #[snafu(display("could not connect to Unix Stream because : {}", source))]
    ActorDied {
        source: tokio::sync::oneshot::error::RecvError,
    },
    #[snafu(display("IO error because : {}", source))]
    IO { source: tokio::io::Error },
    #[snafu(display("could not connect to Unix Stream because : {}", source))]
    SerdeJson { source: serde_json::Error },
}
