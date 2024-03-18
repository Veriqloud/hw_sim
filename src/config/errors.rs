use std::io;

use snafu::prelude::*;

#[derive(Debug, Snafu)]
#[snafu(visibility(pub))]
pub enum Error {
    #[snafu(display("could not read server config at path: {path}"))]
    ReadConfig { source: io::Error, path: String },
    #[snafu(display("could not parse server config: {source}"))]
    ParseConfig { source: serde_json::Error },
}
