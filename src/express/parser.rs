//! Parser for the subset of EXPRESS that record layout depends on.

use std::collections::HashMap;

use super::lexer::{Kind, Tok, tokenize};
use super::schema::{
    AggregateKind, Attribute, AttributeRef, Entity, Redeclaration, Schema, TypeDef, TypeRef,
};
use crate::error::{Error, Result};

pub(crate) fn parse(src: &[u8]) -> Result<Schema> {
    Parser {
        src,
        tokens: tokenize(src)?,
        pos: 0,
    }
    .schema()
}

struct Parser<'a> {
    src: &'a [u8],
    tokens: Vec<Tok>,
    pos: usize,
}

enum AttributeName {
    Plain(String),
    Redeclared(AttributeRef),
}

#[derive(Clone, Copy)]
enum Section {
    Explicit,
    Derive,
    Skip,
}

impl Parser<'_> {
    fn schema(mut self) -> Result<Schema> {
        self.expect_keyword("SCHEMA")?;
        let name = self.ident("a schema name")?;
        if self.peek().is_some_and(|t| t.kind == Kind::String) {
            self.pos += 1;
        }
        self.expect_symbol(b';')?;

        let mut entities = Vec::new();
        let mut types = HashMap::new();
        loop {
            const EXPECTED: &str = "a declaration or END_SCHEMA";
            let token = self.next(EXPECTED)?;
            match self.word(token).as_deref() {
                Some("ENTITY") => entities.push(self.entity()?),
                Some("TYPE") => {
                    let (name, def) = self.type_decl()?;
                    types.insert(name, def);
                }
                Some("FUNCTION") => self.skip_block("FUNCTION", "END_FUNCTION")?,
                Some("PROCEDURE") => self.skip_block("PROCEDURE", "END_PROCEDURE")?,
                Some("RULE") => self.skip_block("RULE", "END_RULE")?,
                Some("SUBTYPE_CONSTRAINT") => {
                    self.skip_block("SUBTYPE_CONSTRAINT", "END_SUBTYPE_CONSTRAINT")?;
                }
                Some("CONSTANT") => self.skip_block("CONSTANT", "END_CONSTANT")?,
                Some("USE" | "REFERENCE") => self.skip_past(b';')?,
                Some("END_SCHEMA") => {
                    self.expect_symbol(b';')?;
                    break;
                }
                _ => return Err(self.unexpected(token, EXPECTED)),
            }
        }
        Ok(Schema::new(name, entities, types))
    }

    fn entity(&mut self) -> Result<Entity> {
        let mut entity = Entity {
            name: self.ident("an entity name")?,
            is_abstract: false,
            supertypes: Vec::new(),
            attributes: Vec::new(),
            redeclared: Vec::new(),
            derived: Vec::new(),
        };

        // Supertype and subtype clauses, up to the `;` ending the header.
        loop {
            const EXPECTED: &str = "ABSTRACT, SUPERTYPE, SUBTYPE or ';'";
            let token = self.next(EXPECTED)?;
            if self.is_symbol(token, b';') {
                break;
            }
            match self.word(token).as_deref() {
                Some("ABSTRACT") => entity.is_abstract = true,
                Some("SUPERTYPE") => {
                    if self.eat_keyword("OF") {
                        self.skip_parenthesised()?;
                    }
                }
                Some("SUBTYPE") => {
                    self.expect_keyword("OF")?;
                    self.expect_symbol(b'(')?;
                    loop {
                        entity.supertypes.push(self.ident("a supertype name")?);
                        let separator = self.next("',' or ')'")?;
                        if self.is_symbol(separator, b')') {
                            break;
                        }
                        if !self.is_symbol(separator, b',') {
                            return Err(self.unexpected(separator, "',' or ')'"));
                        }
                    }
                }
                _ => return Err(self.unexpected(token, EXPECTED)),
            }
        }

        let mut section = Section::Explicit;
        loop {
            let token = self
                .peek()
                .ok_or_else(|| self.end_of_input("an attribute or END_ENTITY"))?;
            match self.word(token).as_deref() {
                Some("END_ENTITY") => {
                    self.pos += 1;
                    self.expect_symbol(b';')?;
                    return Ok(entity);
                }
                Some("DERIVE") => {
                    self.pos += 1;
                    section = Section::Derive;
                    continue;
                }
                Some("INVERSE" | "UNIQUE" | "WHERE") => {
                    self.pos += 1;
                    section = Section::Skip;
                    continue;
                }
                _ => {}
            }
            match section {
                Section::Explicit => self.explicit_attributes(&mut entity)?,
                Section::Derive => {
                    if let AttributeName::Redeclared(reference) = self.attribute_name()? {
                        entity.derived.push(reference);
                    }
                    self.skip_past(b';')?;
                }
                Section::Skip => self.skip_past(b';')?,
            }
        }
    }

    /// `a, b : [OPTIONAL] type;`, where each name may be a redeclaration.
    fn explicit_attributes(&mut self, entity: &mut Entity) -> Result<()> {
        let mut names = Vec::new();
        loop {
            names.push(self.attribute_name()?);
            let separator = self.next("',' or ':'")?;
            if self.is_symbol(separator, b':') {
                break;
            }
            if !self.is_symbol(separator, b',') {
                return Err(self.unexpected(separator, "',' or ':'"));
            }
        }
        let optional = self.eat_keyword("OPTIONAL");
        let ty = self.type_ref()?;
        self.expect_symbol(b';')?;
        for name in names {
            match name {
                AttributeName::Plain(name) => entity.attributes.push(Attribute {
                    name,
                    optional,
                    ty: ty.clone(),
                }),
                AttributeName::Redeclared(attribute) => entity.redeclared.push(Redeclaration {
                    attribute,
                    optional,
                    ty: ty.clone(),
                }),
            }
        }
        Ok(())
    }

    /// `name` or `SELF\entity.attribute [RENAMED name]`.
    fn attribute_name(&mut self) -> Result<AttributeName> {
        let token = self.next("an attribute name")?;
        if !self.is_keyword(token, "SELF") {
            return self
                .word(token)
                .map(AttributeName::Plain)
                .ok_or_else(|| self.unexpected(token, "an attribute name"));
        }
        self.expect_symbol(b'\\')?;
        let entity = self.ident("a supertype name")?;
        self.expect_symbol(b'.')?;
        let attribute = self.ident("an attribute name")?;
        if self.eat_keyword("RENAMED") {
            self.ident("the new attribute name")?;
        }
        Ok(AttributeName::Redeclared(AttributeRef {
            entity,
            attribute,
        }))
    }

    /// `TYPE name = underlying; [WHERE ...] END_TYPE;` after `TYPE`.
    fn type_decl(&mut self) -> Result<(String, TypeDef)> {
        let name = self.ident("a type name")?;
        self.expect_symbol(b'=')?;
        let constructed = self.peek().is_some_and(|t| {
            self.is_keyword(t, "EXTENSIBLE")
                || self.is_keyword(t, "SELECT")
                || self.is_keyword(t, "ENUMERATION")
        });
        let def = if constructed {
            self.skip_past(b';')?;
            TypeDef::Constructed
        } else {
            let ty = self.type_ref()?;
            self.expect_symbol(b';')?;
            TypeDef::Alias(ty)
        };
        loop {
            let token = self.next("END_TYPE")?;
            if self.is_keyword(token, "END_TYPE") {
                self.expect_symbol(b';')?;
                return Ok((name, def));
            }
        }
    }

    fn type_ref(&mut self) -> Result<TypeRef> {
        let token = self.next("a type")?;
        let Some(word) = self.word(token) else {
            return Err(self.unexpected(token, "a type"));
        };
        let kind = match word.as_str() {
            "ARRAY" => Some(AggregateKind::Array),
            "BAG" => Some(AggregateKind::Bag),
            "LIST" => Some(AggregateKind::List),
            "SET" => Some(AggregateKind::Set),
            "AGGREGATE" => Some(AggregateKind::Aggregate),
            _ => None,
        };
        if let Some(kind) = kind {
            if kind == AggregateKind::Aggregate && self.eat_symbol(b':') {
                self.ident("a type label")?;
            }
            let (lower, upper) = if self.eat_symbol(b'[') {
                self.bounds()?
            } else {
                (Some(0), None)
            };
            self.expect_keyword("OF")?;
            while self.eat_keyword("OPTIONAL") || self.eat_keyword("UNIQUE") {}
            return Ok(TypeRef::Aggregate {
                kind,
                lower,
                upper,
                element: Box::new(self.type_ref()?),
            });
        }
        match word.as_str() {
            "STRING" | "BINARY" | "REAL" => {
                if self.eat_symbol(b'(') {
                    self.skip_rest_of_parentheses()?;
                }
                self.eat_keyword("FIXED");
                Ok(TypeRef::Builtin(word))
            }
            "GENERIC" | "GENERIC_ENTITY" => {
                if self.eat_symbol(b':') {
                    self.ident("a type label")?;
                }
                Ok(TypeRef::Builtin(word))
            }
            "INTEGER" | "NUMBER" | "LOGICAL" | "BOOLEAN" => Ok(TypeRef::Builtin(word)),
            _ => Ok(TypeRef::Named(word)),
        }
    }

    /// `lower : upper ]` after `[`, keeping bounds that are literals.
    fn bounds(&mut self) -> Result<(Option<u64>, Option<u64>)> {
        let lower = self.bound(b':')?;
        let upper = self.bound(b']')?;
        Ok((lower, upper))
    }

    /// A bound up to and including `terminator`: its value if it is a lone
    /// integer literal, otherwise `None`.
    fn bound(&mut self, terminator: u8) -> Result<Option<u64>> {
        let start = self.pos;
        self.skip_past(terminator)?;
        let tokens = &self.tokens[start..self.pos - 1];
        Ok(match tokens {
            [token] if token.kind == Kind::Integer => {
                std::str::from_utf8(&self.src[token.start..token.end])
                    .ok()
                    .and_then(|digits| digits.parse().ok())
            }
            _ => None,
        })
    }

    /// Skips from after `begin` through `end ;`, allowing `begin` to nest.
    fn skip_block(&mut self, begin: &str, end: &str) -> Result<()> {
        let mut depth = 1usize;
        loop {
            let token = self.next(end)?;
            if self.is_keyword(token, begin) {
                depth += 1;
            } else if self.is_keyword(token, end) {
                depth -= 1;
                if depth == 0 {
                    return self.expect_symbol(b';');
                }
            }
        }
    }

    /// Skips through the next `symbol` outside parentheses and brackets.
    fn skip_past(&mut self, symbol: u8) -> Result<()> {
        let expected = match symbol {
            b';' => "';'",
            b':' => "':'",
            b']' => "']'",
            _ => "a closing symbol",
        };
        let mut depth = 0usize;
        loop {
            let token = self.next(expected)?;
            if token.kind != Kind::Symbol {
                continue;
            }
            let byte = self.src[token.start];
            if depth == 0 && byte == symbol {
                return Ok(());
            }
            match byte {
                b'(' | b'[' => depth += 1,
                b')' | b']' => depth = depth.saturating_sub(1),
                _ => {}
            }
        }
    }

    fn skip_parenthesised(&mut self) -> Result<()> {
        self.expect_symbol(b'(')?;
        self.skip_rest_of_parentheses()
    }

    /// Skips through the `)` closing an already consumed `(`.
    fn skip_rest_of_parentheses(&mut self) -> Result<()> {
        let mut depth = 1usize;
        loop {
            let token = self.next("')'")?;
            if self.is_symbol(token, b'(') {
                depth += 1;
            } else if self.is_symbol(token, b')') {
                depth -= 1;
                if depth == 0 {
                    return Ok(());
                }
            }
        }
    }

    fn peek(&self) -> Option<Tok> {
        self.tokens.get(self.pos).copied()
    }

    fn next(&mut self, expected: &str) -> Result<Tok> {
        let token = self.peek().ok_or_else(|| self.end_of_input(expected))?;
        self.pos += 1;
        Ok(token)
    }

    fn text(&self, token: Tok) -> &[u8] {
        &self.src[token.start..token.end]
    }

    fn word(&self, token: Tok) -> Option<String> {
        (token.kind == Kind::Ident)
            .then(|| String::from_utf8_lossy(self.text(token)).to_ascii_uppercase())
    }

    fn is_keyword(&self, token: Tok, keyword: &str) -> bool {
        token.kind == Kind::Ident && self.text(token).eq_ignore_ascii_case(keyword.as_bytes())
    }

    fn is_symbol(&self, token: Tok, symbol: u8) -> bool {
        token.kind == Kind::Symbol && self.text(token) == [symbol]
    }

    fn eat_keyword(&mut self, keyword: &str) -> bool {
        let found = self.peek().is_some_and(|t| self.is_keyword(t, keyword));
        if found {
            self.pos += 1;
        }
        found
    }

    fn eat_symbol(&mut self, symbol: u8) -> bool {
        let found = self.peek().is_some_and(|t| self.is_symbol(t, symbol));
        if found {
            self.pos += 1;
        }
        found
    }

    fn expect_keyword(&mut self, keyword: &str) -> Result<()> {
        let token = self.next(keyword)?;
        if self.is_keyword(token, keyword) {
            Ok(())
        } else {
            Err(self.unexpected(token, keyword))
        }
    }

    fn expect_symbol(&mut self, symbol: u8) -> Result<()> {
        let expected = format!("'{}'", char::from(symbol));
        let token = self.next(&expected)?;
        if self.is_symbol(token, symbol) {
            Ok(())
        } else {
            Err(self.unexpected(token, &expected))
        }
    }

    fn ident(&mut self, expected: &str) -> Result<String> {
        let token = self.next(expected)?;
        self.word(token)
            .ok_or_else(|| self.unexpected(token, expected))
    }

    fn unexpected(&self, token: Tok, expected: &str) -> Error {
        const SHOWN: usize = 40;
        let text = self.text(token);
        let shown = String::from_utf8_lossy(&text[..text.len().min(SHOWN)]);
        Error::syntax(
            self.src,
            token.start,
            format!("expected {expected}, found '{shown}'"),
        )
    }

    fn end_of_input(&self, expected: &str) -> Error {
        Error::syntax(
            self.src,
            self.src.len(),
            format!("unexpected end of schema; expected {expected}"),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::express::Slot;

    /// Hand-written; exercises the constructs real long-form schemas use.
    const DEMO: &str = r"
SCHEMA demo_schema '{ 1 2 3 }';
(* An embedded remark (* that nests *) before the declarations. *)
CONSTANT
  origin : REAL := 0.0;
END_CONSTANT;

TYPE label = STRING (255) FIXED; END_TYPE;
TYPE nonempty_items = SET [1:?] OF item; END_TYPE;
TYPE item = EXTENSIBLE GENERIC_ENTITY SELECT (point, line); END_TYPE;
TYPE kind = ENUMERATION OF (a, b);
WHERE
  wr1 : TRUE;
END_TYPE;

ENTITY representation_item
  ABSTRACT SUPERTYPE OF (ONEOF (point, line) ANDOR group);
  name : label;
END_ENTITY;

ENTITY named_unit;
  dimensions : dimensional_exponents;
END_ENTITY;

ENTITY si_unit
  SUBTYPE OF (named_unit);
  prefix : OPTIONAL si_prefix;
  name   : si_unit_name;
DERIVE
  SELF\named_unit.dimensions : dimensional_exponents := dimensions_for_si_unit(name);
END_ENTITY;

ENTITY point
  SUBTYPE OF (representation_item);
  coordinates : LIST [1:3] OF REAL; -- a tail remark with (* in it
END_ENTITY;

ENTITY line
  SUBTYPE OF (representation_item);
  pnt, dir : point;
END_ENTITY;

ENTITY group
  SUBTYPE OF (point, line);
  items : nonempty_items;
  tags  : OPTIONAL SET [0:?] OF UNIQUE label;
  sized : ARRAY [1:hi_index(items)] OF OPTIONAL label;
INVERSE
  users : SET [0:?] OF line FOR pnt;
UNIQUE
  ur1 : items;
WHERE
  wr1 : SIZEOF(QUERY(x <* items | 'DEMO.POINT' IN TYPEOF(x))) >= 0;
END_ENTITY;

ENTITY narrowed
  SUBTYPE OF (group);
  SELF\group.tags : SET [2:?] OF label;
  SELF\representation_item.name RENAMED title : label;
END_ENTITY;

SUBTYPE_CONSTRAINT exclusive FOR representation_item;
  ONEOF (point, line);
END_SUBTYPE_CONSTRAINT;

FUNCTION dimensions_for_si_unit (n : si_unit_name) : dimensional_exponents;
  FUNCTION inner : BOOLEAN;
    RETURN (TRUE);
  END_FUNCTION;
  IF n = 'metre; END_ENTITY; END_FUNCTION;' THEN
    RETURN (?);
  END_IF;
  RETURN (?);
END_FUNCTION;

RULE only_points FOR (point);
WHERE
  wr1 : TRUE;
END_RULE;

END_SCHEMA;
";

    fn schema() -> Schema {
        parse(DEMO.as_bytes()).unwrap()
    }

    fn names(slots: &[Slot]) -> Vec<&str> {
        slots.iter().map(|s| s.name.as_str()).collect()
    }

    #[test]
    fn declarations() {
        let schema = schema();
        assert_eq!(schema.name(), "DEMO_SCHEMA");
        assert!(schema.matches("demo_schema { 1 2 3 }"));
        assert!(!schema.matches("OTHER_SCHEMA"));
        let entities: Vec<&str> = schema.entities().iter().map(|e| e.name.as_str()).collect();
        assert_eq!(
            entities,
            [
                "REPRESENTATION_ITEM",
                "NAMED_UNIT",
                "SI_UNIT",
                "POINT",
                "LINE",
                "GROUP",
                "NARROWED"
            ]
        );
        assert!(schema.entity("representation_item").unwrap().is_abstract);
        assert!(!schema.entity("Point").unwrap().is_abstract);
        assert_eq!(
            schema.entity("group").unwrap().supertypes,
            ["POINT", "LINE"]
        );
        assert!(matches!(
            schema.type_def("kind"),
            Some(TypeDef::Constructed)
        ));
        assert!(matches!(
            schema.type_def("label"),
            Some(TypeDef::Alias(TypeRef::Builtin(name))) if name == "STRING"
        ));
        assert!(schema.entity("nothing").is_none());
    }

    #[test]
    fn inherited_attributes_come_first_and_diamonds_count_once() {
        let schema = schema();
        assert_eq!(
            names(&schema.slots("point").unwrap()),
            ["NAME", "COORDINATES"]
        );
        assert_eq!(
            names(&schema.slots("line").unwrap()),
            ["NAME", "PNT", "DIR"]
        );
        assert_eq!(
            names(&schema.slots("group").unwrap()),
            [
                "NAME",
                "COORDINATES",
                "PNT",
                "DIR",
                "ITEMS",
                "TAGS",
                "SIZED"
            ]
        );
        assert_eq!(
            names(&schema.slots("narrowed").unwrap()),
            names(&schema.slots("group").unwrap()),
            "redeclarations add no attributes"
        );
        assert!(schema.slots("nothing").is_none());
    }

    #[test]
    fn aggregate_bounds() {
        let schema = schema();
        let min = |entity: &str, attribute: &str| {
            schema
                .slots(entity)
                .unwrap()
                .into_iter()
                .find(|s| s.name == attribute)
                .unwrap()
                .min_len
        };
        assert_eq!(min("point", "COORDINATES"), Some(1));
        assert_eq!(min("group", "ITEMS"), Some(1), "through a defined type");
        assert_eq!(min("group", "TAGS"), Some(0));
        assert_eq!(min("group", "SIZED"), Some(1));
        assert_eq!(min("group", "NAME"), None, "not an aggregate");
        assert_eq!(
            min("narrowed", "TAGS"),
            Some(2),
            "narrowed by redeclaration"
        );
    }

    #[test]
    fn derived_redeclarations() {
        let schema = schema();
        let si = schema.slots("si_unit").unwrap();
        assert_eq!(names(&si), ["DIMENSIONS", "PREFIX", "NAME"]);
        assert!(si[0].derived);
        assert!(si[1].optional && !si[1].derived);
        assert!(!schema.slots("named_unit").unwrap()[0].derived);

        let complex = schema.complex_slots(&["NAMED_UNIT", "SI_UNIT"]).unwrap();
        assert_eq!(names(&complex[0]), ["DIMENSIONS"]);
        assert!(complex[0][0].derived, "derived by the SI_UNIT partial");
        assert_eq!(names(&complex[1]), ["PREFIX", "NAME"]);
        assert!(schema.complex_slots(&["NAMED_UNIT", "NOTHING"]).is_none());
    }

    #[test]
    fn malformed_schemas_are_rejected() {
        for (src, needle) in [
            ("ENTITY a; END_ENTITY;", "expected SCHEMA"),
            (
                "SCHEMA s; ENTITY a SUBTYPE OF (b; END_ENTITY; END_SCHEMA;",
                "',' or ')'",
            ),
            (
                "SCHEMA s; ENTITY a; x y; END_ENTITY; END_SCHEMA;",
                "',' or ':'",
            ),
            (
                "SCHEMA s; ENTITY a; x : ; END_ENTITY; END_SCHEMA;",
                "expected a type",
            ),
            (
                "SCHEMA s; ENTITY a; END_ENTITY;",
                "unexpected end of schema",
            ),
            (
                "SCHEMA s; FUNCTION f; END_SCHEMA;",
                "unexpected end of schema",
            ),
            ("SCHEMA s; (* never closed", "unterminated remark"),
        ] {
            let message = parse(src.as_bytes()).unwrap_err().to_string();
            assert!(message.contains(needle), "{src}: {message}");
        }
    }

    #[test]
    fn errors_have_positions() {
        let src = "SCHEMA s;\nENTITY a;\n  x : SET [1:?] product;\nEND_ENTITY;\nEND_SCHEMA;";
        let Err(Error::Syntax { line, column, .. }) = parse(src.as_bytes()) else {
            panic!("expected a syntax error");
        };
        assert_eq!((line, column), (3, 17));
    }
}
