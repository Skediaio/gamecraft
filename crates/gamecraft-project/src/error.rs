//! The error type of projects and packs.

use std::fmt;

/// Something wrong with a pack, a file of one, or a request against a project.
///
/// [`Display`](fmt::Display) gives a sentence an agent can act on: it names the file (as
/// `location/path`), the pack, the id or the field, and what is allowed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectError {
    /// The sentence.
    pub message: String,
}

impl ProjectError {
    /// An error with this sentence.
    pub fn new(message: impl Into<String>) -> ProjectError {
        ProjectError {
            message: message.into(),
        }
    }

    /// An error about the file `file`: `` `file`: message ``.
    pub(crate) fn in_file(file: &str, message: impl fmt::Display) -> ProjectError {
        ProjectError::new(format!("`{file}`: {message}"))
    }
}

impl fmt::Display for ProjectError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ProjectError {}
