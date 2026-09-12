//! ISO 10303-21 ("Part 21") clear-text encoding.
//!
//! This module owns the lexical and syntactic layer only: turning bytes
//! into tokens, tokens into entity instances, and instances back into
//! bytes. It knows nothing about what the entities *mean* — that lives
//! in [`crate::model`].
//!
//! Design constraints (see `docs/ARCHITECTURE.md`):
//!
//! * Files can contain single lines of ~1 MB. The lexer must not be
//!   line-oriented.
//! * `#` inside string literals is not a reference. The lexer must
//!   tokenise strings before anything scans for references.
//! * Complex (multi-inheritance) instances `#1=(A(..)B(..));`, `$`
//!   (unset) and `*` (derived) are first-class, not edge cases.
//! * The writer must reproduce the original attribute text verbatim
//!   for unchanged entities — no reformatting of numbers or strings.
//!
//! Implementation status: the [`Lexer`], [`decode_string`], [`parse`] and
//! [`Writer`] are done. See `ROADMAP.md`.

mod exchange;
mod lexer;
mod parser;
mod string;
mod writer;

pub use exchange::{
    Exchange, Instance, Literal, Param, Params, Record, Records, References, Section,
};
pub use lexer::{Lexer, Span, Token, TokenKind};
pub use parser::parse;
pub use string::decode_string;
pub use writer::{Numbering, Writer};
