use snafu::prelude::*;
use std::io;

#[derive(Debug, Snafu)]
#[snafu(visibility(pub))]
pub enum Error {
    #[snafu(display("Could not create FIFO at '{path}': {source}"))]
    FifoCreation { source: io::Error, path: String },
    #[snafu(display("Could not create or setup mock MMIO file at '{path}': {source}"))]
    MockMmioFileSetup { source: io::Error, path: String },
}