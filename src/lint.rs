//! Structural problems in a STEP file, found without a geometry kernel.
//!
//! This is what `stepq lint` prints. Every check works on the entity graph
//! and, when a schema is available, on the attribute layout the schema
//! declares. Nothing is evaluated.
//!
//! Errors are violations of Part 21 or of the schema that receiving systems
//! may reject or silently misread. Warnings are omissions that conformant
//! readers tolerate but recommended practice asks for.

use std::borrow::Cow;
use std::collections::HashSet;
use std::fmt;

use crate::Error;
use crate::express::{ProblemKind, Schema, check};
use crate::model::Graph;
use crate::p21::{Exchange, Record, parse};

/// The result of [`lint`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct Report {
    /// `FILE_SCHEMA` identifiers from the header.
    pub file_schemas: Vec<String>,
    /// The schema records were checked against, if one matched.
    pub checked_schema: Option<String>,
    /// Findings, file-level ones first, then by instance name.
    pub findings: Vec<Finding>,
}

impl Report {
    /// Number of findings with [`Severity::Error`].
    pub fn errors(&self) -> usize {
        self.count(Severity::Error)
    }

    /// Number of findings with [`Severity::Warning`].
    pub fn warnings(&self) -> usize {
        self.count(Severity::Warning)
    }

    fn count(&self, severity: Severity) -> usize {
        self.findings
            .iter()
            .filter(|finding| finding.severity == severity)
            .count()
    }
}

/// One problem.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct Finding {
    /// How serious it is.
    pub severity: Severity,
    /// Which check found it.
    pub check: Check,
    /// The instance at fault, `#id`; `None` for the file as a whole.
    pub instance: Option<u64>,
    /// What is wrong, without the instance name.
    pub message: String,
}

/// How serious a finding is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "lowercase"))]
pub enum Severity {
    /// The file violates Part 21 or its schema.
    Error,
    /// The file omits something recommended practice asks for.
    Warning,
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // `pad`, not `write_str`, so that width and alignment flags apply.
        f.pad(match self {
            Self::Error => "error",
            Self::Warning => "warning",
        })
    }
}

/// The checks `lint` runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "kebab-case"))]
#[non_exhaustive]
pub enum Check {
    /// The file is not well-formed Part 21. Nothing else is checked.
    Syntax,
    /// An instance name is defined twice. Nothing else is checked.
    DuplicateId,
    /// A reference to an instance that does not exist.
    DanglingReference,
    /// An entity type the schema does not define.
    UnknownEntity,
    /// A record with a different number of attributes than the schema
    /// declares.
    AttributeCount,
    /// A list shorter than its aggregate's lower bound, such as an empty
    /// `SET [1:?]`.
    TooFewElements,
    /// A `representation_relationship_with_transformation` whose two
    /// representations share one context, which its WHERE rule forbids.
    SharedContext,
    /// No `application_protocol_definition`.
    MissingApplicationProtocol,
    /// A `product` in no `product_related_product_category`.
    UncategorizedProduct,
}

impl Check {
    /// The check's name, as `stepq lint` prints it.
    pub fn name(self) -> &'static str {
        match self {
            Self::Syntax => "syntax",
            Self::DuplicateId => "duplicate-id",
            Self::DanglingReference => "dangling-reference",
            Self::UnknownEntity => "unknown-entity",
            Self::AttributeCount => "attribute-count",
            Self::TooFewElements => "too-few-elements",
            Self::SharedContext => "shared-context",
            Self::MissingApplicationProtocol => "missing-application-protocol",
            Self::UncategorizedProduct => "uncategorized-product",
        }
    }

    /// How serious this check's findings are.
    pub fn severity(self) -> Severity {
        match self {
            Self::MissingApplicationProtocol | Self::UncategorizedProduct => Severity::Warning,
            _ => Severity::Error,
        }
    }
}

impl fmt::Display for Check {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.pad(self.name())
    }
}

/// Checks the file in `src`.
///
/// Records are checked against the first of `schemas` that the header's
/// `FILE_SCHEMA` names; with no match, schema checks are skipped and
/// [`Report::checked_schema`] is `None`. A file that does not parse yields
/// a single [`Check::Syntax`] or [`Check::DuplicateId`] finding.
pub fn lint(src: &[u8], schemas: &[Schema]) -> Report {
    let exchange = match parse(src) {
        Ok(exchange) => exchange,
        Err(error) => return unparsable(&error),
    };
    let file_schemas = file_schemas(&exchange);
    let schema = schemas
        .iter()
        .find(|schema| file_schemas.iter().any(|name| schema.matches(name)));
    let graph = Graph::build(exchange);
    let mut findings = Vec::new();

    for &(node, to) in graph.unresolved() {
        findings.push(finding(
            Check::DanglingReference,
            Some(graph.instance(node).id),
            format!("references undefined instance #{to}"),
        ));
    }
    if let Some(schema) = schema {
        schema_findings(schema, graph.exchange(), &mut findings);
    }
    structure_findings(graph.exchange(), &mut findings);

    // Stable: within an instance, checks keep the order they ran in.
    findings.sort_by_key(|finding| finding.instance);
    Report {
        file_schemas,
        checked_schema: schema.map(|schema| schema.name().to_owned()),
        findings,
    }
}

fn finding(check: Check, instance: Option<u64>, message: String) -> Finding {
    Finding {
        severity: check.severity(),
        check,
        instance,
        message,
    }
}

fn unparsable(error: &Error) -> Report {
    let (check, instance) = match error {
        Error::DuplicateId(id) => (Check::DuplicateId, Some(*id)),
        _ => (Check::Syntax, None),
    };
    let message = match error {
        Error::DuplicateId(_) => "is defined more than once".to_owned(),
        _ => error.to_string(),
    };
    Report {
        file_schemas: Vec::new(),
        checked_schema: None,
        findings: vec![finding(check, instance, message)],
    }
}

fn file_schemas(exchange: &Exchange<'_>) -> Vec<String> {
    exchange
        .header_entity("FILE_SCHEMA")
        .and_then(|record| record.param(0))
        .and_then(|param| param.list())
        .into_iter()
        .flatten()
        .filter_map(|param| param.literal())
        .filter_map(|literal| literal.decode().ok())
        .map(Cow::into_owned)
        .collect()
}

fn schema_findings(schema: &Schema, exchange: &Exchange<'_>, findings: &mut Vec<Finding>) {
    for problem in check(schema, exchange) {
        let check = match problem.kind {
            ProblemKind::UnknownEntity => Check::UnknownEntity,
            ProblemKind::AttributeCount { .. } => Check::AttributeCount,
            ProblemKind::TooFewElements { .. } => Check::TooFewElements,
        };
        // The problem's own text starts with `#id `; the id is kept apart.
        let text = problem.to_string();
        let message = text
            .split_once(' ')
            .map_or_else(|| text.clone(), |(_, rest)| rest.to_owned());
        findings.push(finding(check, Some(problem.instance), message));
    }
}

fn structure_findings(exchange: &Exchange<'_>, findings: &mut Vec<Finding>) {
    let mut has_application_protocol = false;
    let mut products = Vec::new();
    let mut categorized = HashSet::new();

    for instance in exchange.instances() {
        let records: Vec<Record<'_>> = exchange.records(instance).collect();
        for record in &records {
            if record.is("APPLICATION_PROTOCOL_DEFINITION") {
                has_application_protocol = true;
            } else if record.is("PRODUCT") {
                products.push(instance.id);
            } else if record.is("PRODUCT_RELATED_PRODUCT_CATEGORY") {
                let listed = record.param(2).and_then(|param| param.list());
                categorized.extend(listed.into_iter().flatten().filter_map(|p| p.reference()));
            }
        }

        let Some((rep_1, rep_2)) = transformed_representations(instance.is_complex(), &records)
        else {
            continue;
        };
        if let (Some(context_1), Some(context_2)) =
            (context_of(exchange, rep_1), context_of(exchange, rep_2))
        {
            if context_1 == context_2 {
                findings.push(finding(
                    Check::SharedContext,
                    Some(instance.id),
                    format!(
                        "representations #{rep_1} and #{rep_2} share context #{context_1}; \
                         a transformation needs two distinct contexts"
                    ),
                ));
            }
        }
    }

    if !has_application_protocol && !exchange.instances().is_empty() {
        findings.push(finding(
            Check::MissingApplicationProtocol,
            None,
            "no APPLICATION_PROTOCOL_DEFINITION; readers cannot tell which protocol \
             the file conforms to"
                .to_owned(),
        ));
    }
    for product in products {
        if !categorized.contains(&product) {
            findings.push(finding(
                Check::UncategorizedProduct,
                Some(product),
                "PRODUCT is in no PRODUCT_RELATED_PRODUCT_CATEGORY".to_owned(),
            ));
        }
    }
}

/// `rep_1` and `rep_2` of a `representation_relationship_with_transformation`.
fn transformed_representations(complex: bool, records: &[Record<'_>]) -> Option<(u64, u64)> {
    let base = if complex {
        if !records
            .iter()
            .any(|r| r.is("REPRESENTATION_RELATIONSHIP_WITH_TRANSFORMATION"))
        {
            return None;
        }
        records
            .iter()
            .find(|r| r.is("REPRESENTATION_RELATIONSHIP"))?
    } else {
        let record = records.first()?;
        if !record.is("REPRESENTATION_RELATIONSHIP_WITH_TRANSFORMATION") {
            return None;
        }
        record
    };
    Some((base.param(2)?.reference()?, base.param(3)?.reference()?))
}

/// `context_of_items` of the representation `id`.
fn context_of(exchange: &Exchange<'_>, id: u64) -> Option<u64> {
    let instance = exchange.get(id)?;
    let mut records = exchange.records(instance);
    let record = if instance.is_complex() {
        records.find(|r| r.is("REPRESENTATION"))?
    } else {
        records.next()?
    };
    record.param(2)?.reference()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file_with(data: &str) -> String {
        format!(
            "ISO-10303-21;HEADER;FILE_SCHEMA(('DEMO'));ENDSEC;DATA;{data}ENDSEC;END-ISO-10303-21;"
        )
    }

    fn summary(report: &Report) -> Vec<(Check, Option<u64>)> {
        report
            .findings
            .iter()
            .map(|finding| (finding.check, finding.instance))
            .collect()
    }

    #[test]
    fn a_clean_file_has_no_findings() {
        let report = lint(
            file_with(
                "#1=APPLICATION_PROTOCOL_DEFINITION('','demo',2026,#2);
#2=APPLICATION_CONTEXT('');
#3=PRODUCT('p','p','',());
#4=PRODUCT_RELATED_PRODUCT_CATEGORY('part','',(#3));",
            )
            .as_bytes(),
            &[],
        );
        assert_eq!(report.findings, []);
        assert_eq!(report.file_schemas, ["DEMO"]);
        assert_eq!(report.checked_schema, None);
    }

    #[test]
    fn structural_problems_are_found_without_a_schema() {
        let report = lint(
            file_with(
                "#1=PRODUCT('p','p','',());
#2=PRODUCT('q','q','',());
#3=PRODUCT_RELATED_PRODUCT_CATEGORY('part','',(#2));
#10=GEOMETRIC_REPRESENTATION_CONTEXT(3);
#11=GEOMETRIC_REPRESENTATION_CONTEXT(3);
#20=SHAPE_REPRESENTATION('',(),#10);
#21=SHAPE_REPRESENTATION('',(),#10);
#22=(REPRESENTATION('',(),#11)SHAPE_REPRESENTATION());
#30=(REPRESENTATION_RELATIONSHIP('','',#20,#21)REPRESENTATION_RELATIONSHIP_WITH_TRANSFORMATION(#40)SHAPE_REPRESENTATION_RELATIONSHIP());
#31=REPRESENTATION_RELATIONSHIP_WITH_TRANSFORMATION('','',#20,#22,#40);
#32=REPRESENTATION_RELATIONSHIP_WITH_TRANSFORMATION('','',#21,#20,#40);
#40=ITEM_DEFINED_TRANSFORMATION('','',#99,#99);",
            )
            .as_bytes(),
            &[],
        );
        assert_eq!(
            summary(&report),
            [
                (Check::MissingApplicationProtocol, None),
                (Check::UncategorizedProduct, Some(1)),
                (Check::SharedContext, Some(30)),
                (Check::SharedContext, Some(32)),
                (Check::DanglingReference, Some(40)),
                (Check::DanglingReference, Some(40)),
            ]
        );
        assert_eq!(report.errors(), 4);
        assert_eq!(report.warnings(), 2);
        assert_eq!(
            report.findings[2].message,
            "representations #20 and #21 share context #10; a transformation needs two distinct contexts"
        );
    }

    #[test]
    fn records_are_checked_against_the_schema_the_header_names() {
        let schema = Schema::parse(
            b"SCHEMA demo;
ENTITY application_context; application : STRING; END_ENTITY;
ENTITY application_protocol_definition;
  status : STRING; name : STRING; year : INTEGER; application : application_context;
END_ENTITY;
ENTITY point; name : STRING; tags : SET [1:?] OF STRING; END_ENTITY;
END_SCHEMA;",
        )
        .unwrap();
        let other = Schema::parse(b"SCHEMA other; ENTITY point; END_ENTITY; END_SCHEMA;").unwrap();
        let src = file_with(
            "#1=APPLICATION_PROTOCOL_DEFINITION('','demo',2026,#2);
#2=APPLICATION_CONTEXT('');
#3=POINT('p',());
#4=POINT('q');
#5=MYSTERY();",
        );
        let report = lint(src.as_bytes(), &[other, schema]);
        assert_eq!(report.checked_schema.as_deref(), Some("DEMO"));
        assert_eq!(
            summary(&report),
            [
                (Check::TooFewElements, Some(3)),
                (Check::AttributeCount, Some(4)),
                (Check::UnknownEntity, Some(5)),
            ]
        );
        assert_eq!(
            report.findings[1].message,
            "POINT: expected 2 attributes, found 1"
        );
    }

    #[test]
    fn unparsable_files_yield_one_finding() {
        let duplicate = lint(file_with("#1=A();#1=B();").as_bytes(), &[]);
        assert_eq!(summary(&duplicate), [(Check::DuplicateId, Some(1))]);

        let broken = lint(b"ISO-10303-21;HEADER;ENDSEC;DATA;#1=A(;", &[]);
        assert_eq!(summary(&broken), [(Check::Syntax, None)]);
        assert!(
            broken.findings[0]
                .message
                .starts_with("syntax error at line 1")
        );
        assert_eq!(broken.errors(), 1);
    }
}
