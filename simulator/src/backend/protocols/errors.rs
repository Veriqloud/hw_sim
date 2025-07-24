use snafu::Snafu;

#[derive(Debug, PartialEq, Snafu)]
pub enum ProtocolError {
    Role { reason: String },
    ModulatorState { reason: String },
    Size { reason: String },
}
