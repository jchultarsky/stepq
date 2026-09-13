//! Tokeniser for EXPRESS source.

use crate::error::{Error, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Ident,
    Integer,
    Real,
    String,
    Symbol,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct Tok {
    pub(crate) kind: Kind,
    pub(crate) start: usize,
    pub(crate) end: usize,
}

/// Splits EXPRESS source into tokens, dropping whitespace, embedded
/// remarks `(* ... *)` (which may nest) and tail remarks `-- ...`.
/// Operators come back one character at a time: the parser only ever
/// skips expressions, so it never needs to interpret them.
pub(crate) fn tokenize(src: &[u8]) -> Result<Vec<Tok>> {
    let mut tokens = Vec::with_capacity(src.len() / 4);
    let mut i = 0;
    while let Some(&byte) = src.get(i) {
        let start = i;
        let kind = match byte {
            _ if byte.is_ascii_whitespace() => {
                i += 1;
                continue;
            }
            b'(' if src.get(i + 1) == Some(&b'*') => {
                i = skip_remark(src, i)?;
                continue;
            }
            b'-' if src.get(i + 1) == Some(&b'-') => {
                i = src[i..]
                    .iter()
                    .position(|&b| b == b'\n')
                    .map_or(src.len(), |n| i + n);
                continue;
            }
            b'\'' => {
                i = end_of_string(src, i)?;
                Kind::String
            }
            b'"' => {
                let Some(n) = src[i + 1..].iter().position(|&b| b == b'"') else {
                    return Err(Error::syntax(src, start, "unterminated encoded string"));
                };
                i += n + 2;
                Kind::String
            }
            b'0'..=b'9' => {
                i = digits(src, i);
                if src.get(i) == Some(&b'.')
                    && src
                        .get(i + 1)
                        .is_some_and(|b| b.is_ascii_digit() || matches!(b, b'e' | b'E'))
                {
                    i = digits(src, i + 1);
                    if matches!(src.get(i), Some(b'e' | b'E')) {
                        i += 1;
                        if matches!(src.get(i), Some(b'+' | b'-')) {
                            i += 1;
                        }
                        i = digits(src, i);
                    }
                    Kind::Real
                } else {
                    Kind::Integer
                }
            }
            b if b.is_ascii_alphabetic() || b == b'_' => {
                i += 1;
                while src
                    .get(i)
                    .is_some_and(|b| b.is_ascii_alphanumeric() || *b == b'_')
                {
                    i += 1;
                }
                Kind::Ident
            }
            _ => {
                i += 1;
                Kind::Symbol
            }
        };
        tokens.push(Tok {
            kind,
            start,
            end: i,
        });
    }
    Ok(tokens)
}

fn digits(src: &[u8], mut i: usize) -> usize {
    while src.get(i).is_some_and(u8::is_ascii_digit) {
        i += 1;
    }
    i
}

fn end_of_string(src: &[u8], start: usize) -> Result<usize> {
    let mut i = start + 1;
    loop {
        let Some(n) = src[i..].iter().position(|&b| b == b'\'') else {
            return Err(Error::syntax(src, start, "unterminated string"));
        };
        i += n + 1;
        if src.get(i) == Some(&b'\'') {
            i += 1;
        } else {
            return Ok(i);
        }
    }
}

fn skip_remark(src: &[u8], start: usize) -> Result<usize> {
    let mut depth = 0usize;
    let mut i = start;
    while i + 1 < src.len() {
        match (src[i], src[i + 1]) {
            (b'(', b'*') => {
                depth += 1;
                i += 2;
            }
            (b'*', b')') => {
                depth -= 1;
                i += 2;
                if depth == 0 {
                    return Ok(i);
                }
            }
            _ => i += 1,
        }
    }
    Err(Error::syntax(src, start, "unterminated remark"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(src: &str) -> Vec<(Kind, &str)> {
        tokenize(src.as_bytes())
            .unwrap()
            .into_iter()
            .map(|t| (t.kind, &src[t.start..t.end]))
            .collect()
    }

    #[test]
    fn tokens_and_remarks() {
        let src = "ENTITY a_1; (* outer (* nested *) still *) x : SET [1:?] OF 'it''s'; -- tail (*\n 2.5E-3 \"0041\" END_ENTITY;";
        assert_eq!(
            texts(src),
            [
                (Kind::Ident, "ENTITY"),
                (Kind::Ident, "a_1"),
                (Kind::Symbol, ";"),
                (Kind::Ident, "x"),
                (Kind::Symbol, ":"),
                (Kind::Ident, "SET"),
                (Kind::Symbol, "["),
                (Kind::Integer, "1"),
                (Kind::Symbol, ":"),
                (Kind::Symbol, "?"),
                (Kind::Symbol, "]"),
                (Kind::Ident, "OF"),
                (Kind::String, "'it''s'"),
                (Kind::Symbol, ";"),
                (Kind::Real, "2.5E-3"),
                (Kind::String, "\"0041\""),
                (Kind::Ident, "END_ENTITY"),
                (Kind::Symbol, ";"),
            ]
        );
    }

    #[test]
    fn index_after_integer_is_not_a_real() {
        assert_eq!(
            texts("a[1].b"),
            [
                (Kind::Ident, "a"),
                (Kind::Symbol, "["),
                (Kind::Integer, "1"),
                (Kind::Symbol, "]"),
                (Kind::Symbol, "."),
                (Kind::Ident, "b"),
            ]
        );
    }

    #[test]
    fn unterminated_input_is_an_error() {
        for src in ["(* open (* nested *)", "'open", "\"0041"] {
            assert!(tokenize(src.as_bytes()).is_err(), "{src}");
        }
    }
}
