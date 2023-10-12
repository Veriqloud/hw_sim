use snafu::Snafu;

#[derive(Debug, Snafu)]
#[snafu(visibility(pub))]
pub enum Error {
    #[snafu(display("could not connect to Unix Stream because : {}", source))]
    UnixStream { source: std::io::Error },
    #[snafu(display("could not connect to Unix Stream because : {}", source))]
    SerdeJson { source: serde_json::Error },
}
