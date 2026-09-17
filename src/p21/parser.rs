//! Parser for the Part 21 exchange structure.
//!
//! Validates the grammar of the whole file and records every instance's
//! tokens into an [`Exchange`]. Parameters are checked for well-formed
//! nesting but not interpreted; that is the model's job.
//!
//! Leniencies, in addition to the lexer's:
//!
//! * the header may hold any number of entities, not exactly three;
//! * section keywords are matched ignoring ASCII case;
//! * edition 3 `ANCHOR` and `REFERENCE` entries are kept as written, one
//!   token list each, and not checked further; `SIGNATURE` sections are
//!   kept as text;
//! * anything after `END-ISO-10303-21;` is ignored.

use std::collections::HashMap;
use std::collections::hash_map::Entry;

use super::exchange::{DataSection, Exchange, ExtraKind, ExtraSection, Instance};
use super::lexer::{Lexer, Span, Token, TokenKind};
use crate::error::{Error, Result};

/// Parameter lists nested deeper than this are rejected. Real files nest
/// a handful of levels; this bounds what crafted input can make us track.
const MAX_DEPTH: usize = 128;

const AFTER_HEADER: &str = "DATA, END-ISO-10303-21 or another section";

/// Parses a complete Part 21 file.
///
/// ```
/// let src = b"ISO-10303-21;HEADER;FILE_SCHEMA(('CONFIG_CONTROL_DESIGN'));ENDSEC;\
///             DATA;#1=PRODUCT('p1','bracket','',(#2));#2=PRODUCT_CONTEXT('',$,'');ENDSEC;\
///             END-ISO-10303-21;";
/// let exchange = stepq::p21::parse(src)?;
/// let product = exchange.get(1).unwrap();
/// assert_eq!(exchange.references(product).collect::<Vec<_>>(), [2]);
/// # Ok::<(), stepq::Error>(())
/// ```
///
/// # Errors
///
/// Returns [`Error::Syntax`] if the file is not well-formed, and
/// [`Error::DuplicateId`] if an instance name is defined more than once.
/// References to undefined instances are not checked here; see
/// [`Graph::new`](crate::model::Graph::new).
pub fn parse(src: &[u8]) -> Result<Exchange<'_>> {
    Parser::new(src).file()
}

struct Parser<'a> {
    src: &'a [u8],
    lexer: Lexer<'a>,
    peeked: Option<Token>,
    tokens: Vec<Token>,
}

#[derive(Clone, Copy)]
enum Frame {
    List,
    Typed,
}

#[derive(Clone, Copy)]
enum Expect {
    ParamOrClose,
    Param,
    CommaOrClose,
}

impl<'a> Parser<'a> {
    fn new(src: &'a [u8]) -> Self {
        Self {
            src,
            lexer: Lexer::new(src),
            peeked: None,
            tokens: Vec::with_capacity(src.len() / 8),
        }
    }

    fn file(mut self) -> Result<Exchange<'a>> {
        self.expect_keyword("ISO-10303-21")?;
        self.expect(TokenKind::Semicolon, "';'")?;
        self.expect_keyword("HEADER")?;
        let header_open = self.expect(TokenKind::Semicolon, "';'")?;

        let header_start = self.tokens.len();
        let header_end = loop {
            if let Some(endsec) = self.eat_keyword("ENDSEC")? {
                break endsec.span.start;
            }
            let token = self.next("a header entity or ENDSEC")?;
            if !matches!(token.kind, TokenKind::Keyword | TokenKind::UserKeyword) {
                return Err(self.unexpected(token, "a header entity or ENDSEC"));
            }
            self.record(token)?;
            self.expect(TokenKind::Semicolon, "';'")?;
        };
        self.expect(TokenKind::Semicolon, "';'")?;
        let header = header_start..self.tokens.len();
        let header_text = Span {
            start: header_open.span.end,
            end: header_end,
        };

        let mut sections = Vec::new();
        let mut extra = Vec::new();
        let mut instances = Vec::new();
        let mut index = HashMap::new();
        loop {
            let token = self.next(AFTER_HEADER)?;
            let extra_kind = [
                ("ANCHOR", ExtraKind::Anchor),
                ("REFERENCE", ExtraKind::Reference),
                ("SIGNATURE", ExtraKind::Signature),
            ]
            .into_iter()
            .find(|(keyword, _)| self.is_keyword(token, keyword))
            .map(|(_, kind)| kind);
            if self.is_keyword(token, "DATA") {
                let section = self.data_section(&mut instances, &mut index)?;
                sections.push(section);
            } else if let Some(kind) = extra_kind {
                extra.push(self.extra_section(token, kind)?);
            } else if self.is_keyword(token, "END-ISO-10303-21") {
                self.expect(TokenKind::Semicolon, "';'")?;
                break;
            } else {
                return Err(self.unexpected(token, AFTER_HEADER));
            }
        }

        Ok(Exchange::new(
            self.src,
            self.tokens,
            header,
            header_text,
            sections,
            instances,
            index,
        )
        .with_extra_sections(extra))
    }

    fn data_section(
        &mut self,
        instances: &mut Vec<Instance>,
        index: &mut HashMap<u64, usize>,
    ) -> Result<DataSection> {
        let params_start = self.tokens.len();
        if self
            .peek()?
            .is_some_and(|token| token.kind == TokenKind::LeftParen)
        {
            let open = self.next("'('")?;
            self.keep(open);
            self.parameters()?;
        }
        let params = params_start..self.tokens.len();
        self.expect(TokenKind::Semicolon, "';'")?;

        let first = instances.len();
        loop {
            let token = self.next("an entity instance or ENDSEC")?;
            match token.kind {
                TokenKind::InstanceName(id) => {
                    let instance = self.instance(token, id)?;
                    match index.entry(id) {
                        Entry::Occupied(_) => return Err(Error::DuplicateId(id)),
                        Entry::Vacant(slot) => {
                            slot.insert(instances.len());
                        }
                    }
                    instances.push(instance);
                }
                _ if self.is_keyword(token, "ENDSEC") => {
                    self.expect(TokenKind::Semicolon, "';'")?;
                    break;
                }
                _ => return Err(self.unexpected(token, "an entity instance or ENDSEC")),
            }
        }
        Ok(DataSection {
            params,
            instances: first..instances.len(),
        })
    }

    fn instance(&mut self, name: Token, id: u64) -> Result<Instance> {
        self.expect(TokenKind::Equals, "'='")?;
        let start = self.tokens.len();
        let token = self.next("an entity record")?;
        let complex = match token.kind {
            TokenKind::Keyword if token.span.slice(self.src).starts_with(b"&") => {
                return Err(Error::syntax(
                    self.src,
                    token.span.start,
                    "scope structures (&SCOPE) are not supported",
                ));
            }
            TokenKind::Keyword | TokenKind::UserKeyword => {
                self.record(token)?;
                false
            }
            TokenKind::LeftParen => {
                self.keep(token);
                let mut records = 0usize;
                loop {
                    let expected = if records == 0 {
                        "an entity record"
                    } else {
                        "an entity record or ')'"
                    };
                    let part = self.next(expected)?;
                    match part.kind {
                        TokenKind::Keyword | TokenKind::UserKeyword => {
                            self.record(part)?;
                            records += 1;
                        }
                        TokenKind::RightParen if records > 0 => {
                            self.keep(part);
                            break;
                        }
                        _ => return Err(self.unexpected(part, expected)),
                    }
                }
                true
            }
            _ => return Err(self.unexpected(token, "an entity record")),
        };
        let tokens = start..self.tokens.len();
        let end = self.expect(TokenKind::Semicolon, "';'")?;
        Ok(Instance {
            id,
            span: Span {
                start: name.span.start,
                end: end.span.end,
            },
            tokens,
            complex,
        })
    }

    /// Parses `NAME(params)` whose name token has just been read.
    fn record(&mut self, name: Token) -> Result<()> {
        self.keep(name);
        let open = self.expect(TokenKind::LeftParen, "'('")?;
        self.keep(open);
        self.parameters()
    }

    /// Parses parameters up to and including the `)` that closes the `(`
    /// just kept. Iterative, so nesting depth cannot overflow the stack.
    fn parameters(&mut self) -> Result<()> {
        let mut stack = vec![Frame::List];
        let mut expect = Expect::ParamOrClose;
        loop {
            let token = self.next(match expect {
                Expect::ParamOrClose => "a parameter or ')'",
                Expect::Param => "a parameter",
                Expect::CommaOrClose => "',' or ')'",
            })?;
            self.keep(token);
            match (expect, token.kind) {
                (Expect::CommaOrClose, TokenKind::Comma) => {
                    if matches!(stack.last(), Some(Frame::Typed)) {
                        return Err(Error::syntax(
                            self.src,
                            token.span.start,
                            "a typed parameter holds exactly one value",
                        ));
                    }
                    expect = Expect::Param;
                }
                (Expect::CommaOrClose | Expect::ParamOrClose, TokenKind::RightParen) => {
                    stack.pop();
                    if stack.is_empty() {
                        return Ok(());
                    }
                    expect = Expect::CommaOrClose;
                }
                (Expect::CommaOrClose, _) => return Err(self.unexpected(token, "',' or ')'")),
                (_, TokenKind::LeftParen) => {
                    self.push(&mut stack, Frame::List, token)?;
                    expect = Expect::ParamOrClose;
                }
                (_, TokenKind::Keyword | TokenKind::UserKeyword) => {
                    let open = self.expect(TokenKind::LeftParen, "'(' after type name")?;
                    self.keep(open);
                    self.push(&mut stack, Frame::Typed, token)?;
                    expect = Expect::Param;
                }
                (
                    _,
                    TokenKind::Dollar
                    | TokenKind::Asterisk
                    | TokenKind::Integer
                    | TokenKind::Real
                    | TokenKind::String
                    | TokenKind::Enumeration
                    | TokenKind::Binary
                    | TokenKind::InstanceName(_)
                    | TokenKind::ConstantName
                    | TokenKind::ValueName
                    | TokenKind::Resource,
                ) => expect = Expect::CommaOrClose,
                _ => return Err(self.unexpected(token, "a parameter")),
            }
        }
    }

    fn push(&self, stack: &mut Vec<Frame>, frame: Frame, token: Token) -> Result<()> {
        if stack.len() >= MAX_DEPTH {
            return Err(Error::syntax(
                self.src,
                token.span.start,
                format!("parameters nested more than {MAX_DEPTH} levels deep"),
            ));
        }
        stack.push(frame);
        Ok(())
    }

    /// An `ANCHOR`, `REFERENCE` or `SIGNATURE` section after its keyword:
    /// each entry up to its `;` is kept as one token list. A signature's
    /// content is not tokenised into entries.
    fn extra_section(&mut self, keyword: Token, kind: ExtraKind) -> Result<ExtraSection> {
        self.expect(TokenKind::Semicolon, "';'")?;
        let mut entries = Vec::new();
        let end = loop {
            if self.eat_keyword("ENDSEC")?.is_some() {
                break self.expect(TokenKind::Semicolon, "';'")?.span.end;
            }
            let first = self.next("an entry or ENDSEC")?;
            if kind == ExtraKind::Signature {
                continue;
            }
            let start = self.tokens.len();
            self.keep(first);
            loop {
                let token = self.next("';'")?;
                if token.kind == TokenKind::Semicolon {
                    let span = Span {
                        start: first.span.start,
                        end: token.span.end,
                    };
                    entries.push((span, start..self.tokens.len()));
                    break;
                }
                self.keep(token);
            }
        };
        Ok(ExtraSection {
            kind,
            span: Span {
                start: keyword.span.start,
                end,
            },
            entries,
        })
    }

    fn keep(&mut self, token: Token) {
        self.tokens.push(token);
    }

    fn peek(&mut self) -> Result<Option<Token>> {
        if self.peeked.is_none() {
            self.peeked = self.lexer.next_token()?;
        }
        Ok(self.peeked)
    }

    fn next(&mut self, expected: &str) -> Result<Token> {
        if let Some(token) = self.peeked.take() {
            return Ok(token);
        }
        self.lexer.next_token()?.ok_or_else(|| {
            Error::syntax(
                self.src,
                self.src.len(),
                format!("unexpected end of file; expected {expected}"),
            )
        })
    }

    fn expect(&mut self, kind: TokenKind, expected: &str) -> Result<Token> {
        let token = self.next(expected)?;
        if token.kind == kind {
            Ok(token)
        } else {
            Err(self.unexpected(token, expected))
        }
    }

    fn expect_keyword(&mut self, keyword: &str) -> Result<()> {
        let token = self.next(keyword)?;
        if self.is_keyword(token, keyword) {
            Ok(())
        } else {
            Err(self.unexpected(token, keyword))
        }
    }

    /// Consumes and returns the next token if it is `keyword`.
    fn eat_keyword(&mut self, keyword: &str) -> Result<Option<Token>> {
        match self.peek()? {
            Some(token) if self.is_keyword(token, keyword) => {
                self.peeked = None;
                Ok(Some(token))
            }
            _ => Ok(None),
        }
    }

    fn is_keyword(&self, token: Token, keyword: &str) -> bool {
        token.kind == TokenKind::Keyword
            && token
                .span
                .slice(self.src)
                .eq_ignore_ascii_case(keyword.as_bytes())
    }

    fn unexpected(&self, token: Token, expected: &str) -> Error {
        const SHOWN: usize = 40;
        let text = token.span.slice(self.src);
        let shown = String::from_utf8_lossy(&text[..text.len().min(SHOWN)]);
        let ellipsis = if text.len() > SHOWN { "..." } else { "" };
        Error::syntax(
            self.src,
            token.span.start,
            format!("expected {expected}, found '{shown}{ellipsis}'"),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::p21::{Param, Record};

    const MINIMAL: &str = "ISO-10303-21;
HEADER;
FILE_DESCRIPTION((''),'2;1');
FILE_NAME('bracket','2026-09-12T00:00:00',(''),(''),'','','');
FILE_SCHEMA(('AUTOMOTIVE_DESIGN'));
ENDSEC;
DATA;
#1=PRODUCT('b','bracket','',(#2));
#2=PRODUCT_CONTEXT('',#3,'mechanical');
#3 = APPLICATION_CONTEXT ( /* c */ 'design' ) ;
#4=(LENGTH_UNIT()NAMED_UNIT(*)SI_UNIT(.MILLI.,.METRE.));
ENDSEC;
END-ISO-10303-21;
";

    fn file_with(data: &str) -> String {
        format!("ISO-10303-21;HEADER;FILE_SCHEMA(('X'));ENDSEC;DATA;{data}ENDSEC;END-ISO-10303-21;")
    }

    fn error(src: &str) -> Error {
        match parse(src.as_bytes()) {
            Err(err) => err,
            Ok(_) => panic!("expected {src:?} to fail"),
        }
    }

    #[test]
    fn header_and_instances() {
        let exchange = parse(MINIMAL.as_bytes()).unwrap();

        let header: Vec<&[u8]> = exchange.header().map(Record::name).collect();
        assert_eq!(
            header,
            [&b"FILE_DESCRIPTION"[..], b"FILE_NAME", b"FILE_SCHEMA"]
        );
        let file_name = exchange.header().find(|r| r.is("file_name")).unwrap();
        let Some(Param::String(name)) = file_name.params().next() else {
            panic!("FILE_NAME starts with a string");
        };
        assert_eq!(name.decode().unwrap(), "bracket");

        let ids: Vec<u64> = exchange.instances().iter().map(|i| i.id).collect();
        assert_eq!(ids, [1, 2, 3, 4]);
        assert_eq!(exchange.sections().len(), 1);

        let context = exchange.get(3).unwrap();
        assert_eq!(
            exchange.text(context),
            b"#3 = APPLICATION_CONTEXT ( /* c */ 'design' ) ;"
        );
        assert!(!context.is_complex());
        assert_eq!(
            exchange
                .references(exchange.get(1).unwrap())
                .collect::<Vec<_>>(),
            [2]
        );

        let unit = exchange.get(4).unwrap();
        assert!(unit.is_complex());
        let parts: Vec<&[u8]> = exchange.records(unit).map(Record::name).collect();
        assert_eq!(parts, [&b"LENGTH_UNIT"[..], b"NAMED_UNIT", b"SI_UNIT"]);
    }

    #[test]
    fn parameter_views() {
        let src = file_with(
            "#1=A('it''s',$,*,-3,2.5,.T.,\"0F\",#1,(1,(2,3),()),LENGTH_MEASURE(5.),#ORIGIN,@V,<x.stp>);",
        );
        let exchange = parse(src.as_bytes()).unwrap();
        let record = exchange.records(&exchange.instances()[0]).next().unwrap();
        let params: Vec<Param<'_>> = record.params().collect();
        assert_eq!(params.len(), 13);

        assert!(matches!(params[0], Param::String(s) if s.decode().unwrap() == "it's"));
        assert!(matches!(params[1], Param::Unset));
        assert!(matches!(params[2], Param::Derived));
        assert!(matches!(params[3], Param::Integer(n) if n.to_i64() == Some(-3)));
        assert!(matches!(params[4], Param::Real(x) if x.to_f64() == Some(2.5)));
        assert!(matches!(params[5], Param::Enumeration(e) if e.enumeration() == Some("T")));
        assert!(matches!(params[6], Param::Binary(b) if b.text() == b"\"0F\""));
        assert!(matches!(params[7], Param::Reference(1)));

        let Param::List(list) = &params[8] else {
            panic!("a list");
        };
        let items: Vec<Param<'_>> = list.clone().collect();
        assert_eq!(items.len(), 3);
        assert!(matches!(&items[1], Param::List(inner) if inner.clone().count() == 2));
        assert!(matches!(&items[2], Param::List(inner) if inner.clone().count() == 0));

        let Param::Typed(typed) = params[9] else {
            panic!("a typed parameter");
        };
        assert!(typed.is("LENGTH_MEASURE"));
        assert!(matches!(typed.params().next(), Some(Param::Real(x)) if x.text() == b"5."));

        assert!(matches!(params[10], Param::ConstantReference(_)));
        assert!(matches!(params[11], Param::ValueReference(_)));
        assert!(matches!(params[12], Param::Resource(_)));
    }

    #[test]
    fn edition_3_anchors_and_references() {
        use crate::p21::{Anchor, ExternalReference, ReferenceName};

        let src = "ISO-10303-21;HEADER;FILE_SCHEMA(('A'));ENDSEC;\
                   ANCHOR;<origin>=#1;<frame>=(#1,#2);ENDSEC;\
                   REFERENCE;#9=<bolt.stp#shape>;#LIB=<lib.stp#x>;@v=<values.stp>;ENDSEC;\
                   DATA;#1=X(#2,#9);#2=Y();ENDSEC;\
                   SIGNATURE;'c2lnbmVk';ENDSEC;\
                   END-ISO-10303-21;";
        let exchange = parse(src.as_bytes()).unwrap();
        assert_eq!(
            exchange.anchors(),
            [
                Anchor {
                    name: "origin",
                    target: Some(1),
                    text: b"<origin>=#1;",
                },
                Anchor {
                    name: "frame",
                    target: None,
                    text: b"<frame>=(#1,#2);",
                },
            ]
        );
        assert_eq!(
            exchange.external_references(),
            [
                ExternalReference {
                    name: ReferenceName::Instance(9),
                    uri: "bolt.stp#shape",
                },
                ExternalReference {
                    name: ReferenceName::Constant("LIB"),
                    uri: "lib.stp#x",
                },
                ExternalReference {
                    name: ReferenceName::Value("v"),
                    uri: "values.stp",
                },
            ]
        );
        assert!(exchange.is_external(9));
        assert!(!exchange.is_external(2));
        // #9 is defined by another file, not dangling.
        let graph = crate::model::Graph::new(exchange).unwrap();
        assert!(graph.unresolved().is_empty());
        assert_eq!(graph.references(0), [1]);
    }

    #[test]
    fn edition_3_sections() {
        let src = "ISO-10303-21;HEADER;FILE_SCHEMA(('A','B'));ENDSEC;\
                   ANCHOR;<origin>=#1;ENDSEC;\
                   DATA('one',('A'));#1=X(#2);ENDSEC;\
                   DATA('two',('B'));#2=Y();ENDSEC;\
                   END-ISO-10303-21;";
        let exchange = parse(src.as_bytes()).unwrap();
        let sections: Vec<_> = exchange.sections().collect();
        assert_eq!(sections.len(), 2);
        assert_eq!(sections[0].instances[0].id, 1);
        assert_eq!(sections[1].instances[0].id, 2);
        assert!(
            matches!(sections[1].params.clone().next(), Some(Param::String(s)) if s.decode().unwrap() == "two")
        );
        assert_eq!(exchange.instances().len(), 2);
    }

    #[test]
    fn malformed_files_are_rejected() {
        for (src, needle) in [
            (
                "HEADER;ENDSEC;".to_owned(),
                "expected ISO-10303-21, found 'HEADER'",
            ),
            (file_with("#1=A(1,);"), "expected a parameter, found ')'"),
            (file_with("#1=A(1 2);"), "expected ',' or ')', found '2'"),
            (file_with("#1=();"), "expected an entity record, found ')'"),
            (file_with("#1=A(B());"), "expected a parameter, found ')'"),
            (file_with("#1=A(B(1,2));"), "exactly one value"),
            (file_with("#1=A(B);"), "expected '(' after type name"),
            (file_with("#1=A(1)"), "expected ';', found 'ENDSEC'"),
            (file_with("#1=A((1);"), "expected ',' or ')', found ';'"),
            (
                file_with("#1=&SCOPE #2=B(); ENDSCOPE A();"),
                "not supported",
            ),
            (file_with("A(1);"), "expected an entity instance or ENDSEC"),
            (file_with("#1 A();"), "expected '='"),
            (
                "ISO-10303-21;HEADER;ENDSEC;DATA;#1=A();".to_owned(),
                "unexpected end of file",
            ),
            (
                "ISO-10303-21;HEADER;ENDSEC;FOO;".to_owned(),
                "expected DATA, END-ISO-10303-21",
            ),
        ] {
            let message = error(&src).to_string();
            assert!(message.contains(needle), "{src:?}: {message}");
        }
    }

    #[test]
    fn errors_report_position() {
        let src = "ISO-10303-21;\nHEADER;\nENDSEC;\nDATA;\n#1=A(1,);\n";
        let Error::Syntax { line, column, .. } = error(src) else {
            panic!("expected a syntax error");
        };
        assert_eq!((line, column), (5, 8));
    }

    #[test]
    fn duplicate_ids_are_rejected() {
        assert!(matches!(
            error(&file_with("#1=A();#2=B();#1=C();")),
            Error::DuplicateId(1)
        ));
    }

    #[test]
    fn nesting_is_bounded() {
        let deep = format!("#1=A({}{});", "(".repeat(200), ")".repeat(200));
        let message = error(&file_with(&deep)).to_string();
        assert!(message.contains("nested more than"), "{message}");

        let fine = format!("#1=A({}{});", "(".repeat(100), ")".repeat(100));
        assert!(parse(file_with(&fine).as_bytes()).is_ok());
    }

    #[test]
    fn leniencies() {
        let src = "iso-10303-21;header;endsec;data;#1=a();endsec;end-iso-10303-21;\n\0\0 trailing";
        assert_eq!(parse(src.as_bytes()).unwrap().instances().len(), 1);
    }
}
