//! Zip errors.
//!
//! thiserror was DELIBERATELY not used: tacet-zip's identity is "zero
//! dependency" — an empty [dependencies] section in the crate's Cargo.toml is an
//! auditable guarantee. The hand-written Display+Error takes 30 lines; that is
//! the price.

use std::fmt;

pub type ZipResult<T> = Result<T, ZipError>;

/// Malformed input is reduced to ONE error kind: the only thing the call site
/// can do anyway is to say "this file could not be opened". The detail (`reason`)
/// is for diagnostics only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ZipError {
    /// The archive structure is inconsistent: no signature, an offset overflows,
    /// the directory is truncated...
    Malformed(&'static str),
    /// The input deliberately pushes the limits (zip bomb, an absurd size).
    LimitExceeded(&'static str),
    /// A compression method we do not know (only STORE and DEFLATE are supported).
    UnsupportedMethod(u16),
    /// CRC32 does not match: the data decoded but is corrupt.
    CrcMismatch { name: String },
}

impl fmt::Display for ZipError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ZipError::Malformed(reason) => write!(f, "the zip file is malformed ({reason})"),
            ZipError::LimitExceeded(reason) => {
                write!(f, "the zip content exceeds the limit ({reason})")
            }
            ZipError::UnsupportedMethod(m) => {
                write!(f, "unsupported compression method: {m}")
            }
            ZipError::CrcMismatch { name } => {
                write!(f, "'{name}' inside the zip could not be verified")
            }
        }
    }
}

impl std::error::Error for ZipError {}
