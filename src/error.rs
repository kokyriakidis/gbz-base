//! The error type used in this crate.
//!
//! [`Error`] is a simple error type that combines an error kind or code ([`ErrorKind`]) with a human-readable message.
//! [`Result`] is a convenient shorthand for [`std::result::Result<T, Error>`].

use std::fmt;
use std::io;

//-----------------------------------------------------------------------------

/// A shorthand for a [`std::result::Result`] with an [`Error`].
pub type Result<T> = std::result::Result<T, Error>;

//-----------------------------------------------------------------------------

/// The kind of an [`Error`].
///
/// The variants are distinguished by the action needed to resolve the error.
/// New variants may be added in the future, so callers should not assume that this list is exhaustive.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum ErrorKind {
    /// An operation in the SQLite layer failed.
    ///
    /// The database file may be missing, locked, or inaccessible.
    /// The query itself is not at fault.
    Database,

    /// A file or stream operation failed.
    ///
    /// Check the file names, permissions, and available disk space.
    Io,

    /// Input data is malformed, or data read from a database is inconsistent.
    ///
    /// The input must be fixed or the database rebuilt.
    InvalidData,

    /// The request is contradictory, incomplete, or otherwise cannot be satisfied.
    ///
    /// Change the query or the command line arguments.
    InvalidQuery,

    /// A well-formed request referred to something that does not exist.
    ///
    /// Check the names and coordinates in the query.
    NotFound,

    /// A safety limit was reached before the operation could finish.
    ///
    /// Raise the limit or narrow the query.
    LimitExceeded,

    /// The file or database is of an unsupported type or version.
    ///
    /// Rebuild the database or use a different version of the tools.
    Unsupported,

    /// An internal invariant was violated.
    ///
    /// This is a bug in this crate and should be reported.
    Internal,
}

impl ErrorKind {
    /// Returns a human-readable description of the kind.
    pub fn as_str(&self) -> &'static str {
        match self {
            ErrorKind::Database => "database error",
            ErrorKind::Io => "I/O error",
            ErrorKind::InvalidData => "invalid data",
            ErrorKind::InvalidQuery => "invalid query",
            ErrorKind::NotFound => "not found",
            ErrorKind::LimitExceeded => "limit exceeded",
            ErrorKind::Unsupported => "unsupported",
            ErrorKind::Internal => "internal error",
        }
    }
}

impl fmt::Display for ErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

//-----------------------------------------------------------------------------

/// An error returned by an operation in this crate.
///
/// An error consists of an [`ErrorKind`] and a message.
/// [`Display`](fmt::Display) writes the message alone, while [`Debug`](fmt::Debug) prefixes it with the kind.
/// Because `fn main() -> Result<(), Error>` reports errors using [`Debug`](fmt::Debug), a binary that
/// propagates an error out of `main` prints the kind along with the message.
///
/// [`io::Error`] and [`rusqlite::Error`] can be converted into this error type using the [`From`] trait.
///
/// # Examples
///
/// ```
/// use gbz_base::{Error, ErrorKind};
///
/// let error = Error::not_found("Cannot find a path covering A:100");
/// assert_eq!(error.kind(), ErrorKind::NotFound);
/// assert_eq!(error.message(), "Cannot find a path covering A:100");
///
/// // `Display` writes the message alone.
/// assert_eq!(error.to_string(), "Cannot find a path covering A:100");
///
/// // `Debug` prefixes it with the kind.
/// assert_eq!(format!("{:?}", error), "not found: Cannot find a path covering A:100");
/// ```
///
/// The kind makes it possible to tell apart failures that need different responses:
///
/// ```
/// use gbz_base::{Error, ErrorKind};
///
/// fn advice(error: &Error) -> &'static str {
///     match error.kind() {
///         ErrorKind::LimitExceeded => "raise the limit or narrow the query",
///         ErrorKind::Database => "check the database file",
///         _ => "see the error message",
///     }
/// }
///
/// let error = Error::limit_exceeded("Found more than 100 new nodes between (14, 0) and (17, 0)");
/// assert_eq!(advice(&error), "raise the limit or narrow the query");
/// ```
#[derive(Clone, PartialEq, Eq)]
pub struct Error {
    kind: ErrorKind,
    message: String,
}

impl Error {
    /// Creates a new error of the given kind with the given message.
    pub fn new(kind: ErrorKind, message: impl ToString) -> Self {
        Error { kind, message: message.to_string() }
    }

    /// Returns the kind of the error.
    #[inline]
    pub fn kind(&self) -> ErrorKind {
        self.kind
    }

    /// Returns the error message.
    #[inline]
    pub fn message(&self) -> &str {
        &self.message
    }

    /// Creates a new [`ErrorKind::Database`] error with the given message.
    pub fn database(message: impl ToString) -> Self {
        Self::new(ErrorKind::Database, message)
    }

    /// Creates a new [`ErrorKind::Io`] error with the given message.
    pub fn io(message: impl ToString) -> Self {
        Self::new(ErrorKind::Io, message)
    }

    /// Creates a new [`ErrorKind::InvalidData`] error with the given message.
    pub fn invalid_data(message: impl ToString) -> Self {
        Self::new(ErrorKind::InvalidData, message)
    }

    /// Creates a new [`ErrorKind::InvalidQuery`] error with the given message.
    pub fn invalid_query(message: impl ToString) -> Self {
        Self::new(ErrorKind::InvalidQuery, message)
    }

    /// Creates a new [`ErrorKind::NotFound`] error with the given message.
    pub fn not_found(message: impl ToString) -> Self {
        Self::new(ErrorKind::NotFound, message)
    }

    /// Creates a new [`ErrorKind::LimitExceeded`] error with the given message.
    pub fn limit_exceeded(message: impl ToString) -> Self {
        Self::new(ErrorKind::LimitExceeded, message)
    }

    /// Creates a new [`ErrorKind::Unsupported`] error with the given message.
    pub fn unsupported(message: impl ToString) -> Self {
        Self::new(ErrorKind::Unsupported, message)
    }

    /// Creates a new [`ErrorKind::Internal`] error with the given message.
    pub fn internal(message: impl ToString) -> Self {
        Self::new(ErrorKind::Internal, message)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

// This is deliberately not derived. Rust reports an error returned from `main` using `Debug`,
// and a derived implementation would quote the message and escape the line breaks in it.
impl fmt::Debug for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.kind, self.message)
    }
}

impl std::error::Error for Error {}

//-----------------------------------------------------------------------------

// Note that there is deliberately no `From<String>` implementation. Several dependencies use
// `String` as their error type, and a blanket conversion would assign an arbitrary kind to all of
// them. Such errors must be converted explicitly, e.g. with `map_err(Error::invalid_data)`.

impl From<rusqlite::Error> for Error {
    fn from(error: rusqlite::Error) -> Self {
        Self::database(error)
    }
}

impl From<io::Error> for Error {
    fn from(error: io::Error) -> Self {
        Self::io(error)
    }
}

//-----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_and_debug() {
        let error = Error::database("unable to open database file");
        assert_eq!(error.to_string(), "unable to open database file", "Invalid Display output");
        assert_eq!(format!("{:?}", error), "database error: unable to open database file", "Invalid Debug output");
    }

    #[test]
    fn multiline_messages() {
        // Multi-line messages must not be escaped, as `main` reports errors using `Debug`.
        let error = Error::invalid_query("first line\nsecond line\n");
        assert_eq!(format!("{:?}", error), "invalid query: first line\nsecond line\n", "Invalid Debug output");
    }

    #[test]
    fn kinds() {
        let errors = [
            (Error::database("x"), ErrorKind::Database),
            (Error::io("x"), ErrorKind::Io),
            (Error::invalid_data("x"), ErrorKind::InvalidData),
            (Error::invalid_query("x"), ErrorKind::InvalidQuery),
            (Error::not_found("x"), ErrorKind::NotFound),
            (Error::limit_exceeded("x"), ErrorKind::LimitExceeded),
            (Error::unsupported("x"), ErrorKind::Unsupported),
            (Error::internal("x"), ErrorKind::Internal),
        ];
        for (error, kind) in errors.iter() {
            assert_eq!(error.kind(), *kind, "Invalid kind for {:?}", error);
            assert_eq!(error.message(), "x", "Invalid message for {:?}", error);
        }
    }

    #[test]
    fn conversions() {
        let error: Error = io::Error::new(io::ErrorKind::NotFound, "no such file").into();
        assert_eq!(error.kind(), ErrorKind::Io, "Invalid kind for an I/O error");
        assert_eq!(error.message(), "no such file", "Invalid message for an I/O error");

        let error: Error = rusqlite::Error::QueryReturnedNoRows.into();
        assert_eq!(error.kind(), ErrorKind::Database, "Invalid kind for a database error");
    }

    #[test]
    fn equality() {
        // `PartialEq` is needed for comparing `Result` objects in tests.
        assert_eq!(Error::io("x"), Error::io("x"), "Identical errors were not equal");
        assert_ne!(Error::io("x"), Error::io("y"), "Errors with different messages were equal");
        assert_ne!(Error::io("x"), Error::database("x"), "Errors with different kinds were equal");
    }
}

//-----------------------------------------------------------------------------
