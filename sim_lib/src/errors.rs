use snafu::Snafu;

#[derive(Debug, PartialEq, Snafu)]
pub enum HardwareError {
    GlobalCounterNotSynced,
    Timeout,
    ModulatorStateNotSupported,
    ResetFifoAtThisGcOverflow,
    FifoOverflow,
    Other { reason: String },
}

#[derive(Debug, PartialEq, Snafu)]
pub enum ProtocolError {
    Role { reason: String },
    ModulatorState { reason: String },
    Size { reason: String },
}
