use snafu::prelude::*;
use std::io;

#[derive(Debug, Snafu)]
#[snafu(visibility(pub(crate)))] // Errors visible within the ipc::config module
pub enum Error {
    #[snafu(display("Could not create FIFO at '{path}': {source}"))]
    FifoCreation { source: io::Error, path: String },
}
