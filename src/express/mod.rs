//! EXPRESS (ISO 10303-11) schemas, read at run time.
//!
//! A STEP file's records only make sense against the schema that defines
//! them: how many attributes `PRODUCT` takes, which of them a subtype has
//! redeclared as derived (`*` in the file), and which lists must not be
//! empty. This module reads that much from a long-form EXPRESS schema and
//! nothing more. See `docs/ARCHITECTURE.md`, "Entities stay untyped".
//!
//! stepq does not ship schemas. ISO permits using its EXPRESS files
//! unmodified for the purposes of the standard but does not clearly permit
//! redistributing them, so they are loaded from disk with
//! [`Schema::parse`]; `tools/fetch-schemas.sh` downloads the ones stepq is
//! tested against.
//!
//! What is read: entities, their supertypes, explicit attributes,
//! redeclarations of inherited attributes (explicit and derived), and
//! defined types, for aggregate bounds. What is skipped: `SELECT` and
//! `ENUMERATION` bodies, `WHERE`, `UNIQUE` and `INVERSE` clauses,
//! constants, functions, procedures, rules and subtype constraints.
//!
//! ```
//! use stepq::express::{Schema, check};
//! use stepq::p21::parse;
//!
//! let schema = Schema::parse(b"
//!     SCHEMA demo;
//!     TYPE label = STRING; END_TYPE;
//!     ENTITY product;
//!       id, name : label;
//!       frame_of_reference : SET [1:?] OF product_context;
//!     END_ENTITY;
//!     ENTITY product_context; END_ENTITY;
//!     END_SCHEMA;")?;
//! let file = parse(b"ISO-10303-21;HEADER;FILE_SCHEMA(('DEMO'));ENDSEC;DATA;
//!     #1=PRODUCT('p1','bracket',());
//!     #2=PRODUCT_CONTEXT();
//!     ENDSEC;END-ISO-10303-21;")?;
//!
//! let problems = check(&schema, &file);
//! assert_eq!(problems.len(), 1);
//! assert_eq!(
//!     problems[0].to_string(),
//!     "#1 PRODUCT.FRAME_OF_REFERENCE: needs at least 1 element, found 0"
//! );
//! # Ok::<(), stepq::Error>(())
//! ```

mod check;
mod lexer;
mod parser;
mod schema;

pub use check::{Problem, ProblemKind, check};
pub use schema::{
    AggregateKind, Attribute, AttributeRef, Entity, Redeclaration, Schema, Slot, TypeDef, TypeRef,
};
