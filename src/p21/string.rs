//! Decoding of Part 21 string literals into Unicode text.

use std::borrow::Cow;

use super::lexer::Span;
use crate::error::{Error, Result};

/// Decodes the string literal covering `span` in `src`.
///
/// `span` must be the span of a [`TokenKind::String`](super::TokenKind::String)
/// token, quotes included. The literal is decoded as follows:
///
/// * `''` is an apostrophe and `\\` is a backslash.
/// * `\X\hh` is the ISO 8859-1 character with hexadecimal code `hh`.
/// * `\X2\hhhh…\X0\` is a run of UTF-16 code units, and
///   `\X4\hhhhhhhh…\X0\` a run of Unicode code points.
/// * `\S\c` is the character `c` with its high bit set, in the code page
///   chosen by the last `\PA\` … `\PI\` directive (ISO 8859-1 by default).
///   Only ISO 8859-1 is supported; `\S\` after selecting another page is
///   an error rather than a silently wrong character.
/// * `\N\` and `\T\` are a line feed and a tab.
/// * Line breaks in the file are not part of the value and are removed.
/// * A backslash that does not start one of these directives is kept
///   literally. Conforming files never contain one, but Windows paths
///   written by real exporters do.
/// * Bytes above `0x7F` are read as UTF-8 where they form valid UTF-8,
///   and as ISO 8859-1 otherwise.
///
/// The result borrows from `src` when nothing needed decoding.
///
/// # Errors
///
/// Returns [`Error::Syntax`] if `span` does not cover a string literal, or
/// if a directive inside it is malformed.
pub fn decode_string(src: &[u8], span: Span) -> Result<Cow<'_, str>> {
    let literal = src.get(span.start..span.end).unwrap_or_default();
    let [b'\'', raw @ .., b'\''] = literal else {
        return Err(Error::syntax(src, span.start, "not a string literal"));
    };
    if !raw.iter().any(|&b| is_special(b)) {
        if let Ok(text) = std::str::from_utf8(raw) {
            return Ok(Cow::Borrowed(text));
        }
    }
    let decoder = Decoder {
        src,
        base: span.start + 1,
        raw,
        out: String::with_capacity(raw.len()),
        page: b'A',
    };
    decoder.run().map(Cow::Owned)
}

fn is_special(b: u8) -> bool {
    matches!(b, b'\\' | b'\'' | b'\r' | b'\n')
}

struct Decoder<'a> {
    src: &'a [u8],
    /// Offset of `raw` within `src`, for error positions.
    base: usize,
    /// The literal without its enclosing quotes.
    raw: &'a [u8],
    out: String,
    /// The ISO 8859 part selected by `\P?\`, as a letter `A` (part 1) to `I`.
    page: u8,
}

impl Decoder<'_> {
    fn run(mut self) -> Result<String> {
        let raw = self.raw;
        let mut i = 0;
        while i < raw.len() {
            i = match raw[i] {
                b'\r' | b'\n' => i + 1,
                b'\'' if raw.get(i + 1) == Some(&b'\'') => {
                    self.out.push('\'');
                    i + 2
                }
                b'\'' => return Err(self.error(i, "unescaped apostrophe in string literal")),
                b'\\' => self.directive(i)?,
                _ => {
                    let end = raw[i..]
                        .iter()
                        .position(|&b| is_special(b))
                        .map_or(raw.len(), |n| i + n);
                    self.push_bytes(&raw[i..end]);
                    end
                }
            };
        }
        Ok(self.out)
    }

    /// Decodes the directive whose backslash is at `i`; returns the index
    /// just past it.
    fn directive(&mut self, i: usize) -> Result<usize> {
        let raw = self.raw;
        match &raw[i + 1..] {
            [b'\\', ..] => {
                self.out.push('\\');
                Ok(i + 2)
            }
            [b'N', b'\\', ..] => {
                self.out.push('\n');
                Ok(i + 3)
            }
            [b'T', b'\\', ..] => {
                self.out.push('\t');
                Ok(i + 3)
            }
            [b'S', b'\\', c, ..] => {
                if self.page != b'A' {
                    let part = self.page - b'A' + 1;
                    return Err(self.error(
                        i,
                        format!("\\S\\ in code page ISO 8859-{part} is not supported"),
                    ));
                }
                if !(b' '..=b'~').contains(c) {
                    return Err(
                        self.error(i, "\\S\\ must be followed by a printable ASCII character")
                    );
                }
                self.out.push(char::from(c + 0x80));
                Ok(i + 4)
            }
            [b'P', page @ b'A'..=b'I', b'\\', ..] => {
                self.page = *page;
                Ok(i + 4)
            }
            [b'X', b'\\', hi, lo, ..] => {
                let c = hex(&[*hi, *lo]).and_then(char::from_u32).ok_or_else(|| {
                    self.error(i, "\\X\\ must be followed by two hexadecimal digits")
                })?;
                self.out.push(c);
                Ok(i + 5)
            }
            [b'X', b'2', b'\\', ..] => self.extended(i, 4),
            [b'X', b'4', b'\\', ..] => self.extended(i, 8),
            _ => {
                self.out.push('\\');
                Ok(i + 1)
            }
        }
    }

    /// Decodes `\X2\` (`width` 4) or `\X4\` (`width` 8) starting at `i`.
    fn extended(&mut self, i: usize, width: usize) -> Result<usize> {
        let raw = self.raw;
        let mut pos = i + 4;
        let mut codes = Vec::new();
        while !raw[pos..].starts_with(b"\\X0\\") {
            let code = raw.get(pos..pos + width).and_then(hex).ok_or_else(|| {
                self.error(
                    pos,
                    format!("expected {width} hexadecimal digits or \\X0\\"),
                )
            })?;
            codes.push(code);
            pos += width;
        }
        let text: Option<String> = if width == 4 {
            let units = codes.iter().filter_map(|&c| u16::try_from(c).ok());
            char::decode_utf16(units)
                .collect::<std::result::Result<_, _>>()
                .ok()
        } else {
            codes.iter().map(|&c| char::from_u32(c)).collect()
        };
        let text =
            text.ok_or_else(|| self.error(i, "invalid Unicode in extended string directive"))?;
        self.out.push_str(&text);
        Ok(pos + 4)
    }

    fn push_bytes(&mut self, bytes: &[u8]) {
        match std::str::from_utf8(bytes) {
            Ok(text) => self.out.push_str(text),
            Err(_) => self.out.extend(bytes.iter().copied().map(char::from)),
        }
    }

    fn error(&self, i: usize, message: impl Into<String>) -> Error {
        Error::syntax(self.src, self.base + i, message)
    }
}

fn hex(digits: &[u8]) -> Option<u32> {
    digits
        .iter()
        .try_fold(0u32, |acc, &d| Some(acc * 16 + char::from(d).to_digit(16)?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::p21::{Lexer, TokenKind};

    fn decode_bytes(src: &[u8]) -> Result<String> {
        let token = Lexer::new(src).next().expect("a token")?;
        assert_eq!(token.kind, TokenKind::String);
        decode_string(src, token.span).map(Cow::into_owned)
    }

    fn decode(literal: &str) -> Result<String> {
        decode_bytes(literal.as_bytes())
    }

    #[test]
    fn plain_text_is_borrowed() {
        let src = b"'Bracket, left'";
        let span = Span {
            start: 0,
            end: src.len(),
        };
        assert!(matches!(
            decode_string(src, span),
            Ok(Cow::Borrowed("Bracket, left"))
        ));
    }

    #[test]
    fn directives_and_escapes() {
        for (literal, expected) in [
            ("''", ""),
            ("'it''s'", "it's"),
            (r"'C:\\parts\\a.stp'", r"C:\parts\a.stp"),
            (r"'C:\parts\a.stp'", r"C:\parts\a.stp"),
            (r"'caf\X\E9'", "café"),
            (r"'caf\S\i'", "café"),
            (r"'\PA\caf\S\i'", "café"),
            (r"'\X2\00E9004C\X0\'", "éL"),
            (r"'\X2\D83DDE00\X0\'", "😀"),
            (r"'\X4\0001F600\X0\ ok'", "😀 ok"),
            (r"'\X2\\X0\'", ""),
            (r"'a\N\b\T\c'", "a\nb\tc"),
            ("'wrapped\r\n text'", "wrapped text"),
            ("'caf\u{e9}'", "café"),
        ] {
            assert_eq!(decode(literal).unwrap(), expected, "{literal}");
        }
    }

    #[test]
    fn latin1_bytes_are_accepted() {
        assert_eq!(decode_bytes(b"'caf\xE9'").unwrap(), "café");
    }

    #[test]
    fn malformed_directives_are_errors() {
        for (literal, needle) in [
            (r"'\X2\00E\X0\'", "4 hexadecimal digits"),
            (r"'\X2\00E9'", "4 hexadecimal digits"),
            (r"'\X4\FFFFFFFF\X0\'", "invalid Unicode"),
            (r"'\X2\DE00\X0\'", "invalid Unicode"),
            (r"'\X\ZZ'", "two hexadecimal digits"),
            (r"'\PB\\S\a'", "ISO 8859-2"),
        ] {
            let message = decode(literal).unwrap_err().to_string();
            assert!(message.contains(needle), "{literal}: {message}");
        }
    }

    #[test]
    fn errors_point_into_the_literal() {
        let Err(Error::Syntax { column, .. }) = decode(r"'\X2\00E\X0\'") else {
            panic!("expected a syntax error");
        };
        assert_eq!(column, 6);
    }

    #[test]
    fn non_string_span_is_an_error() {
        let src = b"#1=A(2);";
        let span = Span { start: 5, end: 6 };
        assert!(decode_string(src, span).is_err());
        let out_of_bounds = Span { start: 4, end: 99 };
        assert!(decode_string(src, out_of_bounds).is_err());
    }
}
