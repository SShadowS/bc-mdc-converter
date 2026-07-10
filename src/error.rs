use std::fmt;

#[derive(Debug)]
pub enum Error {
    Io(std::io::Error),
    Zip(String),
    Decode(String),
    UnsupportedVersion { got: (i32, i32) },
    NoMethodContexts,
    Emit(String),
    Cli(String),
}

pub type Result<T> = std::result::Result<T, Error>;

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io(e) => write!(f, "io error: {e}"),
            Error::Zip(s) => write!(f, "zip error: {s}"),
            Error::Decode(s) => write!(f, "decode error: {s}"),
            Error::UnsupportedVersion { got } => write!(f, "snapshot version {}.{} < 2.0 not supported", got.0, got.1),
            Error::NoMethodContexts => write!(f, "no .mdc entries in snapshot"),
            Error::Emit(s) => write!(f, "emit error: {s}"),
            Error::Cli(s) => write!(f, "usage error: {s}"),
        }
    }
}
impl std::error::Error for Error {}
impl From<std::io::Error> for Error { fn from(e: std::io::Error) -> Self { Error::Io(e) } }
