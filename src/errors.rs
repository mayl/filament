//! Errors generated by the compiler.

use crate::{core::Id, frontend, interval_checking};

/// Standard error type for Calyx errors.
#[allow(clippy::large_enum_variant)]
pub enum Error {
    /// Error while parsing a Calyx program.
    ParseError(pest_consume::Error<frontend::Rule>),
    /// The input file is invalid (does not exist).
    InvalidFile(String),
    /// Failed to write the output
    WriteError(String),

    // The name has not been bound
    Undefined(Id, String),
    /// The name has already been bound.
    AlreadyBound(Id, String),

    /// Failed to prove a fact
    CannotProve(interval_checking::Fact),

    /// A miscellaneous error. Should be replaced with a more precise error.
    #[allow(unused)]
    Misc(String),
}

/// Convience wrapper to represent success or meaningul compiler error.
pub type FilamentResult<T> = std::result::Result<T, Error>;

/// A span of the input program.
impl std::fmt::Debug for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        use Error::*;
        match self {
            AlreadyBound(name, bound_by) => {
                write!(f, "Name `{}' is already bound by {}", name, bound_by)
            }
            Undefined(name, typ) => {
                write!(f, "Undefined {} name: {}", typ, name)
            }
            ParseError(err) => write!(f, "Filament Parser: {}", err),
            InvalidFile(msg) | Misc(msg) | WriteError(msg) => {
                write!(f, "{}", msg)
            }
            CannotProve(fact) => {
                write!(f, "Cannot prove fact {:?}", fact)
            }
        }
    }
}

// Conversions from other error types to our error type so that
// we can use `?` in all the places.

impl From<std::str::Utf8Error> for Error {
    fn from(err: std::str::Utf8Error) -> Self {
        Error::InvalidFile(err.to_string())
    }
}

impl From<pest_consume::Error<frontend::Rule>> for Error {
    fn from(e: pest_consume::Error<frontend::Rule>) -> Self {
        Error::ParseError(e)
    }
}

impl From<std::io::Error> for Error {
    fn from(_e: std::io::Error) -> Self {
        Error::WriteError("IO Error".to_string())
    }
}

impl From<rsmt2::errors::Error> for Error {
    fn from(e: rsmt2::errors::Error) -> Self {
        Error::Misc(format!("SMT Error: {}", e))
    }
}
