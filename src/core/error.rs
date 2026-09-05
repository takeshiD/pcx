use std::fmt;

/// Stable, format-independent classes of failure exposed at the CLI seam.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ErrorCategory {
    /// The requested job or command arguments are invalid.
    Usage,
    /// A source is malformed or internally inconsistent.
    InvalidData,
    /// The requested behavior or representation is not supported.
    Unsupported,
    /// A requested Topic, Point Frame, or other item does not exist.
    NotFound,
    /// A required resource bound cannot be proven or honored.
    Resource,
    /// Local input or output failed.
    Io,
    /// Execution was cancelled by an interrupt.
    Interrupted,
    /// An invariant inside `pcx` was violated.
    Internal,
}

/// A structured failure with a category suitable for deterministic handling.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Error {
    category: ErrorCategory,
    message: String,
}

impl Error {
    /// Construct an error in `category` with human-readable context.
    pub fn new(category: ErrorCategory, message: impl Into<String>) -> Self {
        Self {
            category,
            message: message.into(),
        }
    }

    /// Return the stable failure category.
    pub const fn category(&self) -> ErrorCategory {
        self.category
    }

    /// Return the human-readable diagnostic without its category label.
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for Error {}

/// The result type returned by core contracts.
pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod tests {
    use super::{Error, ErrorCategory};

    #[test]
    fn every_error_category_is_preserved_with_its_diagnostic() {
        let categories = [
            ErrorCategory::Usage,
            ErrorCategory::InvalidData,
            ErrorCategory::Unsupported,
            ErrorCategory::NotFound,
            ErrorCategory::Resource,
            ErrorCategory::Io,
            ErrorCategory::Interrupted,
            ErrorCategory::Internal,
        ];

        for category in categories {
            let error = Error::new(category, "context");
            assert_eq!(error.category(), category);
            assert_eq!(error.message(), "context");
            assert_eq!(error.to_string(), "context");
        }
    }
}
