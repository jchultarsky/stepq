//! Error types shared by the whole crate.

/// A convenient alias for results produced by this crate.
pub type Result<T> = std::result::Result<T, Error>;

/// Every failure `stepq` can report.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// The input is not well-formed ISO 10303-21.
    #[error("syntax error at line {line}, column {column}: {message}")]
    Syntax {
        /// 1-based line of the offending token.
        line: usize,
        /// 1-based column of the offending token, counted in bytes.
        column: usize,
        /// 0-based byte offset of the offending token in the input.
        offset: usize,
        /// What went wrong.
        message: String,
    },

    /// An entity instance references an `#id` that does not exist.
    #[error("entity #{from} references undefined instance #{to}")]
    UnresolvedReference {
        /// The referencing instance.
        from: u64,
        /// The missing instance.
        to: u64,
    },

    /// The same `#id` is declared more than once.
    #[error("entity #{0} is defined more than once")]
    DuplicateId(u64),

    /// An external reference to another file could not be resolved.
    #[error("external reference to {file}: {message}")]
    ExternalReference {
        /// The file referred to.
        file: String,
        /// What went wrong.
        message: String,
    },

    /// An I/O failure while reading or writing a file.
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

impl Error {
    /// Builds an [`Error::Syntax`] located at byte `offset` of `src`.
    ///
    /// Line and column are computed here, on the error path only, so the
    /// lexer does not track them for every byte.
    pub(crate) fn syntax(src: &[u8], offset: usize, message: impl Into<String>) -> Self {
        let offset = offset.min(src.len());
        let before = &src[..offset];
        let line_start = before
            .iter()
            .rposition(|&b| b == b'\n')
            .map_or(0, |i| i + 1);
        Self::Syntax {
            line: before.split(|&b| b == b'\n').count(),
            column: offset - line_start + 1,
            offset,
            message: message.into(),
        }
    }
}
