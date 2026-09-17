//! The parsed, still untyped, form of a Part 21 file.
//!
//! An [`Exchange`] keeps the source bytes and one flat vector of tokens.
//! Each [`Instance`] records its `#id`, the span of its original text and
//! the range of tokens that make up its record or records. Nothing is
//! decoded up front: [`Records`], [`Params`] and [`Literal`] are cheap
//! views that walk the tokens on demand.
//!
//! This is the one parsed representation every command works from, so
//! the views favour general questions — "which instances have this
//! type", "what is attribute 3", "what does this refer to" — over
//! anything specific to one command.

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::ops::Range;

use super::lexer::{Span, Token, TokenKind};
use super::string::decode_string;
use crate::error::Result;

/// A parsed Part 21 file, borrowing its source bytes.
///
/// Produced by [`parse`](super::parse). It is `Send` and `Sync`, so
/// commands may query it from several threads.
pub struct Exchange<'a> {
    src: &'a [u8],
    tokens: Vec<Token>,
    header: Range<usize>,
    header_text: Span,
    sections: Vec<DataSection>,
    instances: Vec<Instance>,
    index: HashMap<u64, usize>,
    extra: Vec<ExtraSection>,
    external: HashSet<u64>,
}

/// The kind of an edition 3 section other than `DATA`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ExtraKind {
    Anchor,
    Reference,
    Signature,
}

/// An edition 3 `ANCHOR`, `REFERENCE` or `SIGNATURE` section.
#[derive(Debug, Clone)]
pub(crate) struct ExtraSection {
    pub(crate) kind: ExtraKind,
    /// From the section keyword through its `ENDSEC;`.
    pub(crate) span: Span,
    /// Each entry: its source text, `;` included, and its tokens, `;`
    /// excluded. Empty for a signature.
    pub(crate) entries: Vec<(Span, Range<usize>)>,
}

/// An edition 3 anchor: a name under which other files can refer to part
/// of this one, such as `<origin>=#12;`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Anchor<'e> {
    /// The anchor name, without its angle brackets.
    pub name: &'e str,
    /// The instance the anchor names, if it names exactly one.
    pub target: Option<u64>,
    /// The whole entry as written.
    pub text: &'e [u8],
}

/// An edition 3 external reference: a name defined by another file, such
/// as `#12=<bolt.stp#shape>;`. Instances of this file may refer to it like
/// any other instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExternalReference<'e> {
    /// The name the reference defines.
    pub name: ReferenceName<'e>,
    /// The resource it stands for, without its angle brackets.
    pub uri: &'e str,
}

/// The name an [`ExternalReference`] defines.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReferenceName<'e> {
    /// An entity instance name, `#12`.
    Instance(u64),
    /// A constant instance name, `#NAME`, without the `#`.
    Constant(&'e str),
    /// A value instance name, `@name`, without the `@`.
    Value(&'e str),
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
        header_text: Span,
        sections: Vec<DataSection>,
        instances: Vec<Instance>,
        index: HashMap<u64, usize>,
    ) -> Self {
        Self {
            src,
            tokens,
            header,
            header_text,
            sections,
            instances,
            index,
            extra: Vec::new(),
            external: HashSet::new(),
        }
    }

    /// Adds the edition 3 sections other than `DATA`, and records the
    /// instance names their `REFERENCE` entries define.
    pub(crate) fn with_extra_sections(mut self, sections: Vec<ExtraSection>) -> Self {
        self.external = sections
            .iter()
            .filter(|section| section.kind == ExtraKind::Reference)
            .flat_map(|section| section.entries.iter())
            .filter_map(|(_, tokens)| match self.tokens.get(tokens.start)?.kind {
                TokenKind::InstanceName(id) => Some(id),
                _ => None,
            })
            .collect();
        self.extra = sections;
        self
    }

    /// The edition 3 anchors, in file order; empty for edition 2 files.
    pub fn anchors(&self) -> Vec<Anchor<'_>> {
        self.extra_entries(ExtraKind::Anchor)
            .filter_map(|(span, tokens)| {
                let name = tokens.first().filter(|t| t.kind == TokenKind::Resource)?;
                let target = match tokens.get(2..) {
                    Some(
                        [
                            Token {
                                kind: TokenKind::InstanceName(id),
                                ..
                            },
                        ],
                    ) => Some(*id),
                    _ => None,
                };
                Some(Anchor {
                    name: inner(name.span.slice(self.src), 1, 1),
                    target,
                    text: span.slice(self.src),
                })
            })
            .collect()
    }

    /// The edition 3 external references, in file order; empty for edition
    /// 2 files.
    pub fn external_references(&self) -> Vec<ExternalReference<'_>> {
        self.extra_entries(ExtraKind::Reference)
            .filter_map(|(_, tokens)| {
                let [lhs, equals, uri] = tokens else {
                    return None;
                };
                if equals.kind != TokenKind::Equals || uri.kind != TokenKind::Resource {
                    return None;
                }
                let text = lhs.span.slice(self.src);
                let name = match lhs.kind {
                    TokenKind::InstanceName(id) => ReferenceName::Instance(id),
                    TokenKind::ConstantName => ReferenceName::Constant(inner(text, 1, 0)),
                    TokenKind::ValueName => ReferenceName::Value(inner(text, 1, 0)),
                    _ => return None,
                };
                Some(ExternalReference {
                    name,
                    uri: inner(uri.span.slice(self.src), 1, 1),
                })
            })
            .collect()
    }

    /// True if instance name `#id` is defined by another file, in an
    /// edition 3 `REFERENCE` section, rather than by this one.
    pub fn is_external(&self, id: u64) -> bool {
        self.external.contains(&id)
    }

    pub(crate) fn extra_sections(&self) -> &[ExtraSection] {
        &self.extra
    }

    pub(crate) fn tokens_in(&self, range: Range<usize>) -> &[Token] {
        self.tokens.get(range).unwrap_or_default()
    }

    fn extra_entries(&self, kind: ExtraKind) -> impl Iterator<Item = (Span, &[Token])> {
        self.extra
            .iter()
            .filter(move |section| section.kind == kind)
            .flat_map(|section| section.entries.iter())
            .map(|(span, tokens)| (*span, self.tokens.get(tokens.clone()).unwrap_or_default()))
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

    /// The first header entity named `name`, ignoring ASCII case.
    pub fn header_entity(&self, name: &str) -> Option<Record<'_>> {
        self.header().find(|record| record.is(name))
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

    /// Positions in [`instances`](Self::instances) of every instance with
    /// a record named `name`, ignoring ASCII case, in file order. A complex
    /// instance matches if any of its partial entities does.
    pub fn instances_of<'e>(&'e self, name: &'e str) -> impl Iterator<Item = usize> {
        self.instances
            .iter()
            .enumerate()
            .filter(move |(_, instance)| self.records(instance).any(|record| record.is(name)))
            .map(|(position, _)| position)
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

    pub(crate) fn instance_tokens(&self, instance: &Instance) -> &[Token] {
        self.tokens.get(instance.tokens.clone()).unwrap_or_default()
    }

    /// The source text between `HEADER;` and `ENDSEC;`, comments included.
    pub(crate) fn header_text(&self) -> &'a [u8] {
        self.header_text.slice(self.src)
    }

    /// Where [`header_text`](Self::header_text) is in the source.
    pub(crate) fn header_span(&self) -> Span {
        self.header_text
    }

    /// The tokens of the header entities.
    pub(crate) fn header_tokens(&self) -> &[Token] {
        self.tokens.get(self.header.clone()).unwrap_or_default()
    }

    /// For each data section: the source text of its `(...)` parameters
    /// (empty for a plain `DATA;`) and its instance positions.
    pub(crate) fn data_sections(&self) -> impl Iterator<Item = (&'a [u8], Range<usize>)> {
        self.sections.iter().map(|section| {
            let tokens = &self.tokens[section.params.clone()];
            let params: &'a [u8] = match (tokens.first(), tokens.last()) {
                (Some(first), Some(last)) => &self.src[first.span.start..last.span.end],
                _ => &[],
            };
            (params, section.instances.clone())
        })
    }
}

/// `text` without `front` bytes at the start and `back` at the end, as a
/// string; empty if that is not UTF-8.
fn inner(text: &[u8], front: usize, back: usize) -> &str {
    text.get(front..text.len().saturating_sub(back))
        .and_then(|bytes| std::str::from_utf8(bytes).ok())
        .unwrap_or_default()
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

    /// The parameter at `index`, counting from 0, or `None` if the record
    /// has fewer parameters.
    pub fn param(self, index: usize) -> Option<Param<'e>> {
        self.params().nth(index)
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

impl<'e> Param<'e> {
    /// The referenced `#id`, if this is a [`Param::Reference`].
    pub fn reference(&self) -> Option<u64> {
        match self {
            Self::Reference(id) => Some(*id),
            _ => None,
        }
    }

    /// The items, if this is a [`Param::List`].
    pub fn list(&self) -> Option<Params<'e>> {
        match self {
            Self::List(items) => Some(items.clone()),
            _ => None,
        }
    }

    /// The typed value's record, if this is a [`Param::Typed`].
    pub fn typed(&self) -> Option<Record<'e>> {
        match self {
            Self::Typed(record) => Some(*record),
            _ => None,
        }
    }

    /// The literal, for integers, reals, strings, enumerations, binaries
    /// and edition 3 constant, value and resource references.
    pub fn literal(&self) -> Option<Literal<'e>> {
        match self {
            Self::Integer(literal)
            | Self::Real(literal)
            | Self::String(literal)
            | Self::Enumeration(literal)
            | Self::Binary(literal)
            | Self::ConstantReference(literal)
            | Self::ValueReference(literal)
            | Self::Resource(literal) => Some(*literal),
            Self::Unset | Self::Derived | Self::Reference(_) | Self::List(_) | Self::Typed(_) => {
                None
            }
        }
    }

    /// True for `$`.
    pub fn is_unset(&self) -> bool {
        matches!(self, Self::Unset)
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::p21::parse;

    #[test]
    fn query_helpers() {
        let src = b"ISO-10303-21;HEADER;FILE_NAME('part');FILE_SCHEMA(('AP242'));ENDSEC;DATA;\
                    #1=PRODUCT('p1','bracket','',(#2));\
                    #2=PRODUCT_CONTEXT('',$,'mechanical');\
                    #3=(NAMED_UNIT(*)PRODUCT());\
                    #4=product('p2','plate','',(#2));\
                    ENDSEC;END-ISO-10303-21;";
        let exchange = parse(src).unwrap();

        assert_eq!(
            exchange.instances_of("PRODUCT").collect::<Vec<_>>(),
            [0, 2, 3]
        );
        assert_eq!(exchange.instances_of("NOTHING").count(), 0);

        let schema = exchange.header_entity("file_schema").unwrap();
        let schemas = schema.param(0).and_then(|p| p.list()).unwrap();
        let names: Vec<String> = schemas
            .filter_map(|p| p.literal()?.decode().ok().map(Cow::into_owned))
            .collect();
        assert_eq!(names, ["AP242"]);

        let product = exchange.records(&exchange.instances()[0]).next().unwrap();
        let name = product.param(1).and_then(|p| p.literal()).unwrap();
        assert_eq!(name.decode().unwrap(), "bracket");
        let contexts = product.param(3).and_then(|p| p.list()).unwrap();
        assert_eq!(
            contexts.filter_map(|p| p.reference()).collect::<Vec<_>>(),
            [2]
        );
        assert!(product.param(4).is_none());

        let context = exchange.records(&exchange.instances()[1]).next().unwrap();
        assert!(context.param(1).unwrap().is_unset());
        assert!(context.param(1).unwrap().literal().is_none());
    }
}
