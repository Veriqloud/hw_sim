use std::io;

use snafu::prelude::*;

#[derive(Debug, Snafu)]
#[snafu(visibility(pub))]
pub enum Error {
    #[snafu(display("could not read server config at path: {path}"))]
    ReadConfig { source: io::Error, path: String },
    #[snafu(display("could not parse server config: {source}"))]
    ParseConfig { source: serde_json::Error },
    #[snafu(display("IO error"))]
    PathNotExist { source: io::Error, path: String },
    #[snafu(display("Could not create FIFO at {path}: {source}"))]
    FifoCreation { source: io::Error, path: String },
}