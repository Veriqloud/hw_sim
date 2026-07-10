use snafu::Snafu;
use std::time::Duration;

#[derive(Debug, Snafu)]
#[snafu(visibility(pub))]
pub enum Error {
    #[snafu(display("I/O error on command file: {}", source))]
    CommandFileIo { source: std::io::Error },
    #[snafu(display("Serde error because : {}", source))]
    SerdeJson { source: serde_json::Error },
    #[snafu(display("Serde error because : {}", reason))]
    Unexpected { reason: String },
    #[snafu(display("Runtime pause requested for {:?}", duration))]
    PauseRequested { duration: Duration },
}
