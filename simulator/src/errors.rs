use snafu::Snafu;

#[derive(Debug, Snafu)]
#[snafu(visibility(pub))]
pub enum Error {
    #[snafu(display("IPC Reader error {}", source))]
    IpcReader {
        source: crate::ipc::reader::errors::Error,
    },
    #[snafu(display("Simulator error {}", source))]
    Backend {
        source: crate::backend::errors::Error,
    },
    #[snafu(display("IPC Writer error {}", source))]
    Writer {
        source: crate::ipc::writer::errors::Error,
    },
    #[snafu(display("UnixStream error {}", source))]
    UnixStream { source: std::io::Error },
    #[snafu(display("IO error {}", source))]
    IO { source: std::io::Error },
    #[snafu(display("SerdeJSON error {}", source))]
    SerdeJson { source: serde_json::Error },
    #[snafu(display("Configuration loading/parsing error: {source}"))]
    ConfigLoad { source: configs::errors::Error },
    #[snafu(display("IPC Configuration error: {source}"))]
    IpcConfig { source: configs::ipc::errors::Error },
    #[snafu(display("Logger initialization error: {source}"))]
    LoggerInitialization {
        source: tracing_subscriber::filter::LevelParseError,
    },
}
