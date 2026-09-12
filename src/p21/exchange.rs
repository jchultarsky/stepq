//! The parsed, still untyped, form of a Part 21 file.
//!
//! An [`Exchange`] keeps the source bytes and one flat vector of tokens.
//! Each [`Instance`] records its `#id`, the span of its original text and
//! the range of tokens that make up its record or records. Nothing is
//! decoded up front: [`Records`], [`Params`] and [`Literal`] are cheap
//! views that walk the tokens on demand.

use std::borrow::Cow;
use std::collections::HashMap;
use std::fmt;
use std::ops::Range;

use super::lexer::{Span, Token, TokenKind};
use super::string::decode_string;
use crate::error::Result;

/// A parsed Part 21 file, borrowing its source bytes.
///
/// Produced by [`parse`](super::parse).
pub struct Exchange<'a> {
    src: &'a [u8],
    tokens: Vec<Token>,
    header: Range<usize>,
    sections: Vec<DataSection>,
    instances: Vec<Instance>,
    index: HashMap<u64, usize>,
}

/// One entity instance from a data section.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Instance {
    /// The instance name: `123` for `#123`.
    pub id: u64,
    /// Source bytes from the `#` of the name through the terminating `;`.
    pub span: Span,
    pub(crate) tokens: Range<usize>,
    pub(crate) complex: bool,
}

impl Instance {
    /// True for a complex instance such as `#1=(A(..)B(..));`.
    pub fn is_complex(&self) -> bool {
        self.complex
    }
}

#[derive(Debug, Clone)]
pub(crate) struct DataSection {
    /// Tokens of the optional `DATA(...)` parameter list, parentheses included.
    pub(crate) params: Range<usize>,
    /// Positions in `Exchange::instances`.
    pub(crate) instances: Range<usize>,
}

/// A data section: its optional parameters and the instances it holds.
#[derive(Debug, Clone)]
pub struct Section<'e> {
    /// The parameters of `DATA(...)`. Empty for a plain `DATA;`, which is
    /// the only form edition 2 allows.
    pub params: Params<'e>,
    /// The instances in this section, in file order.
    pub instances: &'e [Instance],
}

impl<'a> Exchange<'a> {
    pub(crate) fn new(
        src: &'a [u8],
        tokens: Vec<Token>,
        header: Range<usize>,
        sections: Vec<DataSection>,
        instances: Vec<Instance>,
        index: HashMap<u64, usize>,
    ) -> Self {
        Self {
            src,
            tokens,
            header,
            sections,
            instances,
            index,
        }
    }

    /// The source bytes the file was parsed from.
    pub fn source(&self) -> &'a [u8] {
        self.src
    }

    /// The header entities, normally `FILE_DESCRIPTION`, `FILE_NAME` and
    /// `FILE_SCHEMA`, in file order.
    pub fn header(&self) -> Records<'_> {
        Records {
            src: self.src,
            tokens: self.tokens.get(self.header.clone()).unwrap_or_default(),
        }
    }

    /// The data sections, in file order. Edition 2 files have exactly one.
    pub fn sections(&self) -> impl ExactSizeIterator<Item = Section<'_>> {
        self.sections.iter().map(|section| {
            let tokens = &self.tokens[section.params.clone()];
            Section {
                params: Params {
                    src: self.src,
                    tokens: tokens
                        .get(1..tokens.len().saturating_sub(1))
                        .unwrap_or_default(),
                },
                instances: &self.instances[section.instances.clone()],
            }
        })
    }

    /// All instances from all data sections, in file order.
    pub fn instances(&self) -> &[Instance] {
        &self.instances
    }

    /// Looks up instance `#id`.
    pub fn get(&self, id: u64) -> Option<&Instance> {
        self.position(id).map(|i| &self.instances[i])
    }

    /// The position of instance `#id` in [`instances`](Self::instances).
    pub fn position(&self, id: u64) -> Option<usize> {
        self.index.get(&id).copied()
    }

    /// The instance's original text, from `#` through `;`, byte for byte.
    pub fn text(&self, instance: &Instance) -> &'a [u8] {
        instance.span.slice(self.src)
    }

    /// The records of an instance: one for a simple instance, one per
    /// partial entity for a complex instance.
    pub fn records(&self, instance: &Instance) -> Records<'_> {
        let tokens = self.instance_tokens(instance);
        let tokens = if instance.complex {
            tokens
                .get(1..tokens.len().saturating_sub(1))
                .unwrap_or_default()
        } else {
            tokens
        };
        Records {
            src: self.src,
            tokens,
        }
    }

    /// The `#id`s an instance refers to, in order of appearance and
    /// including repeats. Names inside string literals are not references
    /// and are never reported.
    pub fn references(&self, instance: &Instance) -> References<'_> {
        References {
            tokens: self.instance_tokens(instance).iter(),
        }
    }

    fn instance_tokens(&self, instance: &Instance) -> &[Token] {
        self.tokens.get(instance.tokens.clone()).unwrap_or_default()
    }
}

impl fmt::Debug for Exchange<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Exchange")
            .field("bytes", &self.src.len())
            .field("sections", &self.sections.len())
            .field("instances", &self.instances.len())
            .finish_non_exhaustive()
    }
}

/// Iterator over the instance names an instance refers to; see
/// [`Exchange::references`].
#[derive(Debug, Clone)]
pub struct References<'e> {
    tokens: std::slice::Iter<'e, Token>,
}

impl Iterator for References<'_> {
    type Item = u64;

    fn next(&mut self) -> Option<u64> {
        self.tokens.find_map(|token| match token.kind {
            TokenKind::InstanceName(id) => Some(id),
            _ => None,
        })
    }
}

/// Iterator over a sequence of records such as the header or the partial
/// entities of a complex instance.
#[derive(Debug, Clone)]
pub struct Records<'e> {
    src: &'e [u8],
    tokens: &'e [Token],
}

impl<'e> Iterator for Records<'e> {
    type Item = Record<'e>;

    fn next(&mut self) -> Option<Record<'e>> {
        let (&name, rest) = self.tokens.split_first()?;
        let close = matching(rest);
        let params = rest.get(1..close).unwrap_or_default();
        self.tokens = rest.get(close + 1..).unwrap_or_default();
        Some(Record {
            src: self.src,
            name,
            params,
        })
    }
}

/// An entity type name with its parameters, such as `PRODUCT('a','b',$,(#2))`.
#[derive(Debug, Clone, Copy)]
pub struct Record<'e> {
    src: &'e [u8],
    name: Token,
    params: &'e [Token],
}

impl<'e> Record<'e> {
    /// The type name exactly as written, for example `b"PRODUCT"`.
    pub fn name(self) -> &'e [u8] {
        self.name.span.slice(self.src)
    }

    /// True if the type name equals `name`, ignoring ASCII case.
    pub fn is(self, name: &str) -> bool {
        self.name().eq_ignore_ascii_case(name.as_bytes())
    }

    /// The record's parameters, in order.
    pub fn params(self) -> Params<'e> {
        Params {
            src: self.src,
            tokens: self.params,
        }
    }
}

/// Iterator over the parameters of a record or list.
#[derive(Debug, Clone)]
pub struct Params<'e> {
    src: &'e [u8],
    tokens: &'e [Token],
}

impl<'e> Iterator for Params<'e> {
    type Item = Param<'e>;

    fn next(&mut self) -> Option<Param<'e>> {
        loop {
            let (&token, rest) = self.tokens.split_first()?;
            let literal = Literal {
                src: self.src,
                token,
            };
            let param = match token.kind {
                TokenKind::Comma
                | TokenKind::RightParen
                | TokenKind::Semicolon
                | TokenKind::Equals => {
                    // Separators; `)`, `;` and `=` cannot occur in parsed input.
                    self.tokens = rest;
                    continue;
                }
                TokenKind::LeftParen => {
                    let close = matching(self.tokens);
                    let inner = self.tokens.get(1..close).unwrap_or_default();
                    self.tokens = self.tokens.get(close + 1..).unwrap_or_default();
                    return Some(Param::List(Params {
                        src: self.src,
                        tokens: inner,
                    }));
                }
                TokenKind::Keyword | TokenKind::UserKeyword => {
                    let close = matching(rest);
                    let params = rest.get(1..close).unwrap_or_default();
                    self.tokens = rest.get(close + 1..).unwrap_or_default();
                    return Some(Param::Typed(Record {
                        src: self.src,
                        name: token,
                        params,
                    }));
                }
                TokenKind::Dollar => Param::Unset,
                TokenKind::Asterisk => Param::Derived,
                TokenKind::Integer => Param::Integer(literal),
                TokenKind::Real => Param::Real(literal),
                TokenKind::String => Param::String(literal),
                TokenKind::Enumeration => Param::Enumeration(literal),
                TokenKind::Binary => Param::Binary(literal),
                TokenKind::InstanceName(id) => Param::Reference(id),
                TokenKind::ConstantName => Param::ConstantReference(literal),
                TokenKind::ValueName => Param::ValueReference(literal),
                TokenKind::Resource => Param::Resource(literal),
            };
            self.tokens = rest;
            return Some(param);
        }
    }
}

/// One parameter of a record or list.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum Param<'e> {
    /// `$`: an unset optional attribute.
    Unset,
    /// `*`: an attribute redeclared as derived by a subtype.
    Derived,
    /// An integer literal.
    Integer(Literal<'e>),
    /// A real literal.
    Real(Literal<'e>),
    /// A string literal.
    String(Literal<'e>),
    /// An enumeration value such as `.T.`.
    Enumeration(Literal<'e>),
    /// A binary literal.
    Binary(Literal<'e>),
    /// A reference to entity instance `#id`.
    Reference(u64),
    /// An edition 3 constant reference, `#NAME`.
    ConstantReference(Literal<'e>),
    /// An edition 3 value reference, `@name`.
    ValueReference(Literal<'e>),
    /// An edition 3 resource reference, `<...>`.
    Resource(Literal<'e>),
    /// A parenthesised list of parameters.
    List(Params<'e>),
    /// A typed value such as `LENGTH_MEASURE(5.0)`: a record holding
    /// exactly one parameter.
    Typed(Record<'e>),
}

/// A literal parameter value.
#[derive(Debug, Clone, Copy)]
pub struct Literal<'e> {
    src: &'e [u8],
    token: Token,
}

impl<'e> Literal<'e> {
    /// The literal exactly as written, including any quotes, dots or sign.
    pub fn text(self) -> &'e [u8] {
        self.token.span.slice(self.src)
    }

    /// Where the literal is in the source.
    pub fn span(self) -> Span {
        self.token.span
    }

    /// Decodes a string literal; see [`decode_string`](super::decode_string).
    ///
    /// # Errors
    ///
    /// Returns [`Error::Syntax`](crate::Error::Syntax) if this is not a
    /// string literal or it contains a malformed directive.
    pub fn decode(self) -> Result<Cow<'e, str>> {
        decode_string(self.src, self.token.span)
    }

    /// The value of an integer literal; `None` for other literals or if it
    /// does not fit in an `i64`.
    pub fn to_i64(self) -> Option<i64> {
        if self.token.kind != TokenKind::Integer {
            return None;
        }
        std::str::from_utf8(self.text()).ok()?.parse().ok()
    }

    /// The value of an integer or real literal; `None` for other literals.
    pub fn to_f64(self) -> Option<f64> {
        if !matches!(self.token.kind, TokenKind::Integer | TokenKind::Real) {
            return None;
        }
        std::str::from_utf8(self.text()).ok()?.parse().ok()
    }

    /// The value of an enumeration literal without its dots: `"T"` for
    /// `.T.`. `None` for other literals.
    pub fn enumeration(self) -> Option<&'e str> {
        if self.token.kind != TokenKind::Enumeration {
            return None;
        }
        let text = self.text();
        std::str::from_utf8(text.get(1..text.len().saturating_sub(1))?).ok()
    }
}

/// Given tokens starting with `(`, returns the index of the matching `)`,
/// or `tokens.len()` if there is none.
fn matching(tokens: &[Token]) -> usize {
    let mut depth = 0usize;
    for (i, token) in tokens.iter().enumerate() {
        match token.kind {
            TokenKind::LeftParen => depth += 1,
            TokenKind::RightParen => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return i;
                }
            }
            _ => {}
        }
    }
    tokens.len()
}
