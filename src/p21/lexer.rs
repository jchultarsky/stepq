//! Tokeniser for the Part 21 clear-text encoding.
//!
//! The lexer turns a byte slice into a stream of [`Token`]s. It borrows
//! the input and never allocates: a token is a [`TokenKind`] plus the
//! [`Span`] of source bytes it covers, so later stages can copy entity
//! text verbatim. Whitespace and `/* */` comments are skipped.
//!
//! It is not line-oriented. Real files contain single lines close to a
//! megabyte, and string literals may continue across line breaks.
//!
//! Where exporters routinely deviate from the standard, the lexer accepts
//! the input rather than rejecting a file every CAD system reads:
//!
//! * a UTF-8 byte-order mark at the very start is skipped;
//! * every byte from `0x00` to `0x20` counts as whitespace;
//! * any byte is accepted inside a string literal;
//! * keywords and enumeration values may be lower case;
//! * exponents may use a lower-case `e`, and a real may omit the decimal
//!   point when it has an exponent (`1E5`);
//! * hexadecimal digits in binary literals may be lower case.

use std::iter::FusedIterator;

use crate::error::{Error, Result};

/// A half-open range of byte offsets, `start..end`, into the lexer's input.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Span {
    /// Offset of the first byte.
    pub start: usize,
    /// Offset one past the last byte.
    pub end: usize,
}

impl Span {
    /// Returns the bytes this span covers.
    ///
    /// # Panics
    ///
    /// Panics if the span lies outside `src`, which cannot happen for a
    /// span produced by a [`Lexer`] over the same `src`.
    pub fn slice(self, src: &[u8]) -> &[u8] {
        &src[self.start..self.end]
    }
}

/// The kind of a [`Token`].
///
/// Only instance names carry a decoded value. Everything else is read
/// from the token's [`Span`] when needed, so that numbers and strings are
/// never re-printed differently from how the file wrote them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum TokenKind {
    /// A standard keyword: an entity or header type name such as
    /// `PRODUCT`, or a section keyword such as `DATA`, `ENDSEC`,
    /// `ISO-10303-21` or `&SCOPE`.
    Keyword,
    /// A user-defined keyword: `!` followed by a name.
    UserKeyword,
    /// An entity instance name, `#123`.
    InstanceName(u64),
    /// An edition 3 constant instance name, `#NAME`.
    ConstantName,
    /// An edition 3 value instance name, `@name`.
    ValueName,
    /// An edition 3 resource reference, `<...>`.
    Resource,
    /// An integer, with optional sign.
    Integer,
    /// A real number, with optional sign and exponent.
    Real,
    /// A string literal, quotes included. Decode it with
    /// [`decode_string`](super::decode_string).
    String,
    /// An enumeration value such as `.T.` or `.UNSPECIFIED.`.
    Enumeration,
    /// A binary literal such as `"0FF"`, quotes included.
    Binary,
    /// `(`
    LeftParen,
    /// `)`
    RightParen,
    /// `,`
    Comma,
    /// `;`
    Semicolon,
    /// `=`
    Equals,
    /// `$`, an unset optional attribute.
    Dollar,
    /// `*`, a redeclared (derived) attribute.
    Asterisk,
}

/// One lexical token.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Token {
    /// What kind of token this is.
    pub kind: TokenKind,
    /// The source bytes the token was read from, including any quotes,
    /// sign, `#` or `.` delimiters.
    pub span: Span,
}

/// A tokeniser over a complete Part 21 file held in memory.
///
/// Iterating yields tokens until the end of input. The first error ends
/// the iteration: after an `Err`, [`Iterator::next`] returns `None`.
///
/// ```
/// use stepq::p21::{Lexer, TokenKind};
///
/// let src = b"#7=PRODUCT('bracket',$);";
/// let kinds: Vec<TokenKind> = Lexer::new(src).map(|t| t.unwrap().kind).collect();
/// assert_eq!(kinds[0], TokenKind::InstanceName(7));
/// assert_eq!(kinds.len(), 9);
/// ```
#[derive(Debug, Clone)]
pub struct Lexer<'a> {
    src: &'a [u8],
    pos: usize,
    failed: bool,
}

impl<'a> Lexer<'a> {
    /// Creates a lexer over `src`, skipping a leading UTF-8 byte-order mark.
    pub fn new(src: &'a [u8]) -> Self {
        let pos = if src.starts_with(b"\xEF\xBB\xBF") {
            3
        } else {
            0
        };
        Self {
            src,
            pos,
            failed: false,
        }
    }

    /// The input this lexer reads from.
    pub fn source(&self) -> &'a [u8] {
        self.src
    }

    /// The byte offset the next token will be read from.
    pub fn offset(&self) -> usize {
        self.pos
    }

    /// Reads the next token, or `None` at the end of input.
    ///
    /// Unlike the [`Iterator`] implementation, this does not stop after an
    /// error; calling it again after an `Err` has unspecified results.
    pub fn next_token(&mut self) -> Result<Option<Token>> {
        self.skip_trivia()?;
        let start = self.pos;
        let Some(&byte) = self.src.get(start) else {
            return Ok(None);
        };
        let kind = match byte {
            b'(' => self.punct(TokenKind::LeftParen),
            b')' => self.punct(TokenKind::RightParen),
            b',' => self.punct(TokenKind::Comma),
            b';' => self.punct(TokenKind::Semicolon),
            b'=' => self.punct(TokenKind::Equals),
            b'$' => self.punct(TokenKind::Dollar),
            b'*' => self.punct(TokenKind::Asterisk),
            b'#' => self.instance_name()?,
            b'\'' => self.string()?,
            b'"' => self.binary()?,
            b'.' => self.enumeration()?,
            b'+' | b'-' | b'0'..=b'9' => self.number()?,
            b'!' => {
                self.pos += 1;
                self.name(start, "'!'")?;
                TokenKind::UserKeyword
            }
            b'&' => {
                self.pos += 1;
                self.name(start, "'&'")?;
                TokenKind::Keyword
            }
            b'@' => {
                self.pos += 1;
                if self.eat_while(is_name_char) == 0 {
                    return Err(self.error(start, "expected a name after '@'"));
                }
                TokenKind::ValueName
            }
            b'<' => self.resource()?,
            b if is_name_start(b) => {
                self.eat_while(|b| is_name_char(b) || b == b'-');
                TokenKind::Keyword
            }
            _ => return Err(self.error(start, format!("unexpected {}", describe(byte)))),
        };
        Ok(Some(Token {
            kind,
            span: Span {
                start,
                end: self.pos,
            },
        }))
    }

    fn skip_trivia(&mut self) -> Result<()> {
        loop {
            self.eat_while(|b| b <= b' ');
            if !self.src[self.pos..].starts_with(b"/*") {
                return Ok(());
            }
            let body = self.pos + 2;
            match find(&self.src[body..], b"*/") {
                Some(i) => self.pos = body + i + 2,
                None => return Err(self.error(self.pos, "unterminated comment")),
            }
        }
    }

    fn punct(&mut self, kind: TokenKind) -> TokenKind {
        self.pos += 1;
        kind
    }

    fn instance_name(&mut self) -> Result<TokenKind> {
        let start = self.pos;
        self.pos += 1;
        if self.eat_while(|b| b.is_ascii_digit()) > 0 {
            let src = self.src;
            return parse_u64(&src[start + 1..self.pos])
                .map(TokenKind::InstanceName)
                .ok_or_else(|| self.error(start, "instance name does not fit in 64 bits"));
        }
        self.name(start, "'#'")?;
        Ok(TokenKind::ConstantName)
    }

    fn string(&mut self) -> Result<TokenKind> {
        let start = self.pos;
        let mut pos = start + 1;
        loop {
            let Some(i) = self.src[pos..].iter().position(|&b| b == b'\'') else {
                return Err(self.error(start, "unterminated string literal"));
            };
            pos += i + 1;
            // A doubled apostrophe is an escaped apostrophe, not the end.
            if self.src.get(pos) == Some(&b'\'') {
                pos += 1;
            } else {
                break;
            }
        }
        self.pos = pos;
        Ok(TokenKind::String)
    }

    fn binary(&mut self) -> Result<TokenKind> {
        let start = self.pos;
        self.pos += 1;
        let digits = self.eat_while(|b| b.is_ascii_hexdigit());
        if self.peek() != Some(b'"') {
            return Err(self.error(
                self.pos,
                "expected a hexadecimal digit or '\"' in binary literal",
            ));
        }
        if digits == 0 || !matches!(self.src[start + 1], b'0'..=b'3') {
            return Err(self.error(
                start,
                "binary literal must start with a digit 0-3 giving the unused bit count",
            ));
        }
        self.pos += 1;
        Ok(TokenKind::Binary)
    }

    fn enumeration(&mut self) -> Result<TokenKind> {
        let start = self.pos;
        self.pos += 1;
        self.name(start, "'.'")?;
        if self.peek() != Some(b'.') {
            return Err(self.error(start, "unterminated enumeration value"));
        }
        self.pos += 1;
        Ok(TokenKind::Enumeration)
    }

    fn number(&mut self) -> Result<TokenKind> {
        let start = self.pos;
        if matches!(self.peek(), Some(b'+' | b'-')) {
            self.pos += 1;
        }
        if self.eat_while(|b| b.is_ascii_digit()) == 0 {
            return Err(self.error(start, "expected a digit after the sign"));
        }
        let mut kind = TokenKind::Integer;
        if self.peek() == Some(b'.') {
            self.pos += 1;
            self.eat_while(|b| b.is_ascii_digit());
            kind = TokenKind::Real;
        }
        if matches!(self.peek(), Some(b'E' | b'e')) {
            let exponent = self.pos;
            self.pos += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.pos += 1;
            }
            if self.eat_while(|b| b.is_ascii_digit()) == 0 {
                return Err(self.error(exponent, "expected digits in exponent"));
            }
            kind = TokenKind::Real;
        }
        Ok(kind)
    }

    fn resource(&mut self) -> Result<TokenKind> {
        let start = self.pos;
        match self.src[start + 1..].iter().position(|&b| b == b'>') {
            Some(i) => {
                self.pos = start + i + 2;
                Ok(TokenKind::Resource)
            }
            None => Err(self.error(start, "unterminated resource reference")),
        }
    }

    /// Consumes a name (letter or `_`, then letters, digits and `_`) that
    /// must follow the prefix already consumed at `start`.
    fn name(&mut self, start: usize, prefix: &str) -> Result<()> {
        if !self.peek().is_some_and(is_name_start) {
            return Err(self.error(start, format!("expected a name after {prefix}")));
        }
        self.eat_while(is_name_char);
        Ok(())
    }

    fn peek(&self) -> Option<u8> {
        self.src.get(self.pos).copied()
    }

    fn eat_while(&mut self, pred: impl Fn(u8) -> bool) -> usize {
        let start = self.pos;
        while self.peek().is_some_and(&pred) {
            self.pos += 1;
        }
        self.pos - start
    }

    fn error(&self, offset: usize, message: impl Into<String>) -> Error {
        Error::syntax(self.src, offset, message)
    }
}

impl Iterator for Lexer<'_> {
    type Item = Result<Token>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.failed {
            return None;
        }
        match self.next_token() {
            Ok(token) => token.map(Ok),
            Err(err) => {
                self.failed = true;
                Some(Err(err))
            }
        }
    }
}

impl FusedIterator for Lexer<'_> {}

fn is_name_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_'
}

fn is_name_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

fn parse_u64(digits: &[u8]) -> Option<u64> {
    digits.iter().try_fold(0u64, |acc, &d| {
        acc.checked_mul(10)?.checked_add(u64::from(d - b'0'))
    })
}

fn describe(byte: u8) -> String {
    if byte.is_ascii_graphic() {
        format!("character '{}'", char::from(byte))
    } else {
        format!("byte 0x{byte:02X}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use TokenKind as K;

    fn lex(src: &str) -> Vec<Token> {
        Lexer::new(src.as_bytes())
            .collect::<Result<_>>()
            .expect("input lexes")
    }

    fn kinds(src: &str) -> Vec<TokenKind> {
        lex(src).into_iter().map(|t| t.kind).collect()
    }

    fn texts(src: &str) -> Vec<&str> {
        lex(src)
            .iter()
            .map(|t| std::str::from_utf8(t.span.slice(src.as_bytes())).unwrap())
            .collect()
    }

    fn syntax_error(src: &str) -> (usize, usize, String) {
        match Lexer::new(src.as_bytes()).find_map(Result::err) {
            Some(Error::Syntax {
                line,
                column,
                message,
                ..
            }) => (line, column, message),
            other => panic!("expected a syntax error for {src:?}, got {other:?}"),
        }
    }

    #[test]
    fn minimal_file() {
        let src = "ISO-10303-21;\nHEADER;\nFILE_SCHEMA(('AUTOMOTIVE_DESIGN'));\nENDSEC;\n\
                   DATA;\n#1=PRODUCT('a','b',$,(#2));\nENDSEC;\nEND-ISO-10303-21;\n";
        let texts = texts(src);
        assert_eq!(texts[..2], ["ISO-10303-21", ";"]);
        assert_eq!(texts[texts.len() - 2..], ["END-ISO-10303-21", ";"]);
        let data = texts.iter().position(|&t| t == "#1").unwrap();
        assert_eq!(
            texts[data..=data + 14],
            [
                "#1", "=", "PRODUCT", "(", "'a'", ",", "'b'", ",", "$", ",", "(", "#2", ")", ")",
                ";"
            ]
        );
    }

    #[test]
    fn token_kinds() {
        assert_eq!(
            kinds("( ) , ; = $ *"),
            [
                K::LeftParen,
                K::RightParen,
                K::Comma,
                K::Semicolon,
                K::Equals,
                K::Dollar,
                K::Asterisk
            ]
        );
        assert_eq!(
            kinds("#42 #ORIGIN @7 <part.stp#frame>"),
            [
                K::InstanceName(42),
                K::ConstantName,
                K::ValueName,
                K::Resource
            ]
        );
        assert_eq!(
            kinds("PRODUCT !ACME_THING &SCOPE ENDSCOPE"),
            [K::Keyword, K::UserKeyword, K::Keyword, K::Keyword]
        );
        assert_eq!(
            kinds(".T. .UNSPECIFIED. \"0FF\" \"3\""),
            [K::Enumeration, K::Enumeration, K::Binary, K::Binary]
        );
    }

    #[test]
    fn numbers() {
        assert_eq!(kinds("0 42 -7 +3"), [K::Integer; 4]);
        assert_eq!(kinds("0. 1.5 -1.E-5 +2.25E+10 2.5e3 1E5"), [K::Real; 6]);
        assert_eq!(texts("(-1.E-5,3)"), ["(", "-1.E-5", ",", "3", ")"]);
    }

    #[test]
    fn complex_instance() {
        assert_eq!(
            texts("#2=(LENGTH_UNIT()NAMED_UNIT(*)SI_UNIT(.MILLI.,.METRE.));"),
            [
                "#2",
                "=",
                "(",
                "LENGTH_UNIT",
                "(",
                ")",
                "NAMED_UNIT",
                "(",
                "*",
                ")",
                "SI_UNIT",
                "(",
                ".MILLI.",
                ",",
                ".METRE.",
                ")",
                ")",
                ";"
            ]
        );
    }

    #[test]
    fn hash_inside_string_is_not_a_reference() {
        // Real line from the STEP Tools AS1 sample written by CADDS.
        let kinds = kinds("#9=PROPERTY_DEFINITION('volume of #602','',#8);");
        assert!(!kinds.contains(&K::InstanceName(602)));
        assert_eq!(kinds.iter().filter(|&&k| k == K::String).count(), 2);
    }

    #[test]
    fn string_literals() {
        assert_eq!(
            texts(r"'it''s' '' '''' 'a\b'"),
            ["'it''s'", "''", "''''", r"'a\b'"]
        );
        // A literal may run across a line break and may end in an escaped quote.
        assert_eq!(texts("'long\r\nline''' ;"), ["'long\r\nline'''", ";"]);
    }

    #[test]
    fn comments_and_whitespace_are_skipped() {
        let src = "\u{FEFF}/* lead */FILE_NAME(/* name */'x',\t/* multi\nline */'y')\r\n\0";
        assert_eq!(texts(src), ["FILE_NAME", "(", "'x'", ",", "'y'", ")"]);
        assert!(lex("  /**/ \n ").is_empty());
    }

    #[test]
    fn errors_report_position() {
        let (line, column, message) = syntax_error("DATA;\n#1=A('open);\n");
        assert_eq!((line, column), (2, 6));
        assert!(message.contains("unterminated string"), "{message}");

        let (line, column, message) = syntax_error("A;\n  /* never closed");
        assert_eq!((line, column), (2, 3));
        assert!(message.contains("unterminated comment"), "{message}");
    }

    #[test]
    fn malformed_input_is_rejected() {
        for (src, needle) in [
            ("#1=A({);", "unexpected character '{'"),
            ("#1=A(/);", "unexpected character '/'"),
            ("A \u{e9}", "unexpected byte 0xC3"),
            ("(- 1)", "expected a digit after the sign"),
            ("(1.E)", "expected digits in exponent"),
            ("(.T)", "unterminated enumeration"),
            ("(..)", "expected a name after '.'"),
            ("(\"4F\")", "unused bit count"),
            ("(\"0FG\")", "binary literal"),
            ("#99999999999999999999=A();", "64 bits"),
            ("# 1", "expected a name after '#'"),
            ("@ 1", "expected a name after '@'"),
            ("! A", "expected a name after '!'"),
            ("<http://example.com", "unterminated resource"),
        ] {
            let (_, _, message) = syntax_error(src);
            assert!(message.contains(needle), "{src:?}: {message}");
        }
    }

    #[test]
    fn iterator_stops_after_an_error() {
        let mut lexer = Lexer::new(b"A { B C");
        assert!(matches!(lexer.next(), Some(Ok(_))));
        assert!(matches!(lexer.next(), Some(Err(_))));
        assert!(lexer.next().is_none());
        assert!(lexer.next().is_none());
    }

    #[test]
    fn megabyte_single_line() {
        let refs = 200_000;
        let mut src = String::from("#1=POLY_LOOP('',(");
        for i in 0..refs {
            if i > 0 {
                src.push(',');
            }
            src.push_str("#12345");
        }
        src.push_str("));");
        assert!(src.len() > 1_000_000);
        let tokens = lex(&src);
        let refs_seen = tokens
            .iter()
            .filter(|t| t.kind == K::InstanceName(12345))
            .count();
        assert_eq!(refs_seen, refs);
    }
}
