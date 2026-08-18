//! Serde error type for XML (de)serialization.

use std::fmt;

/// Error type for serde XML operations.
#[derive(Debug)]
pub enum Error {
    /// A serde serialization/deserialization error.
    Message(String),
    /// An XML parsing error.
    Parse(crate::error::ParseError),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Message(msg) => write!(f, "{msg}"),
            Self::Parse(e) => write!(f, "XML parse error: {e}"),
        }
    }
}

impl std::error::Error for Error {}

impl serde::de::Error for Error {
    fn custom<T: fmt::Display>(msg: T) -> Self {
        Self::Message(msg.to_string())
    }
}

impl serde::ser::Error for Error {
    fn custom<T: fmt::Display>(msg: T) -> Self {
        Self::Message(msg.to_string())
    }
}

impl From<crate::error::ParseError> for Error {
    fn from(e: crate::error::ParseError) -> Self {
        Self::Parse(e)
    }
}
