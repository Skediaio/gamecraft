//! The error of a command.

use std::fmt;

/// Why a command did not run. Every variant holds a sentence an agent can act on.
///
/// The session puts the command id in front of every message of a known command
/// (`entity.add: bad parameters: missing `position` or `at``), so [`Display`](fmt::Display)
/// shows the sentence as it is. The variant says what kind of problem it is.
#[derive(Clone, Debug, PartialEq)]
pub enum CommandError {
    /// No command has this id: `unknown command `x`; did you mean `y`?`.
    Unknown(String),
    /// The parameters do not fit the command's spec, or a value is out of range.
    BadParams(String),
    /// The parameters are fine but the command cannot do it in this state of the world.
    Failed(String),
}

impl CommandError {
    /// The message, without the kind.
    pub fn message(&self) -> &str {
        match self {
            Self::Unknown(m) | Self::BadParams(m) | Self::Failed(m) => m,
        }
    }

    /// The same error with the command id in front: `<command>: bad parameters: <why>` or
    /// `<command>: <why>`.
    pub(crate) fn for_command(self, command: &str) -> CommandError {
        match self {
            Self::Unknown(m) => Self::Unknown(m),
            Self::BadParams(m) => Self::BadParams(format!("{command}: bad parameters: {m}")),
            Self::Failed(m) => Self::Failed(format!("{command}: {m}")),
        }
    }
}

impl fmt::Display for CommandError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.message())
    }
}

impl std::error::Error for CommandError {}

/// A [`CommandError::BadParams`] with this reason.
pub(crate) fn bad(why: impl Into<String>) -> CommandError {
    CommandError::BadParams(why.into())
}

/// A [`CommandError::Failed`] with this reason.
pub(crate) fn failed(why: impl fmt::Display) -> CommandError {
    CommandError::Failed(why.to_string())
}
