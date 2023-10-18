use snafu::Snafu;

#[derive(Debug, PartialEq, Snafu)]
pub enum Error {
    CurrentTimeUnavailable,
}
