//! A summary of a STEP file: header, units, counts and entity types.
//!
//! This is what `stepq info` prints. It is computed from the entity graph
//! alone, without evaluating geometry, and lives in the library so that
//! other front ends can reuse it.

use std::borrow::Cow;
use std::collections::HashMap;

use crate::model::Graph;
use crate::p21::{Exchange, Instance, Param, Record};

/// Summary of a parsed file; see [`Info::new`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct Info {
    /// Size of the source in bytes.
    pub bytes: usize,
    /// Decoded header fields.
    pub header: Header,
    /// Number of data sections.
    pub sections: usize,
    /// Number of entity instances.
    pub instances: usize,
    /// How many of those instances are complex.
    pub complex_instances: usize,
    /// Instances with a `PRODUCT` record.
    pub products: usize,
    /// `NEXT_ASSEMBLY_USAGE_OCCURRENCE` instances: one per component
    /// placed in an assembly. Zero for a single part.
    pub assembly_usages: usize,
    /// References to instances that are not defined.
    pub unresolved_references: usize,
    /// Units the file declares.
    pub units: Units,
    /// Record counts per entity type, most frequent first, then by name.
    /// Names are upper-cased. A complex instance counts once for each of
    /// its partial entities.
    pub entity_types: Vec<EntityCount>,
}

/// Decoded header fields. Empty strings are omitted.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct Header {
    /// `FILE_DESCRIPTION.description`.
    pub description: Vec<String>,
    /// `FILE_DESCRIPTION.implementation_level`, such as `2;1`.
    pub implementation_level: Option<String>,
    /// `FILE_NAME.name`.
    pub name: Option<String>,
    /// `FILE_NAME.time_stamp`.
    pub time_stamp: Option<String>,
    /// `FILE_NAME.author`.
    pub author: Vec<String>,
    /// `FILE_NAME.organization`.
    pub organization: Vec<String>,
    /// `FILE_NAME.preprocessor_version`.
    pub preprocessor_version: Option<String>,
    /// `FILE_NAME.originating_system`.
    pub originating_system: Option<String>,
    /// `FILE_NAME.authorization`.
    pub authorization: Option<String>,
    /// `FILE_SCHEMA.schema_identifiers`.
    pub schemas: Vec<String>,
}

/// Units declared in the file, lower-cased, each listed once in file
/// order: `millimetre` for an SI unit with a prefix, or the name of a
/// conversion-based unit such as `inch` or `degree`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct Units {
    /// Length units.
    pub length: Vec<String>,
    /// Plane angle units.
    pub plane_angle: Vec<String>,
    /// Solid angle units.
    pub solid_angle: Vec<String>,
}

/// How many records of one entity type a file contains.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct EntityCount {
    /// Upper-cased entity type name.
    pub name: String,
    /// Number of records.
    pub count: usize,
}

impl Info {
    /// Summarises the file behind `graph`.
    ///
    /// Build the graph with [`Graph::build`] to summarise files that have
    /// dangling references; they are counted, not fatal.
    pub fn new(graph: &Graph<'_>) -> Self {
        let exchange = graph.exchange();
        let mut raw_counts: HashMap<&[u8], usize> = HashMap::new();
        let mut complex_instances = 0;
        let mut units = Units::default();
        for instance in exchange.instances() {
            if instance.is_complex() {
                complex_instances += 1;
            }
            for record in exchange.records(instance) {
                *raw_counts.entry(record.name()).or_default() += 1;
            }
            collect_unit(&mut units, exchange, instance);
        }

        // Merge spellings that differ only in case.
        let mut counts: HashMap<String, usize> = HashMap::new();
        for (name, count) in raw_counts {
            *counts
                .entry(String::from_utf8_lossy(name).to_ascii_uppercase())
                .or_default() += count;
        }
        let count_of = |name: &str| counts.get(name).copied().unwrap_or(0);
        let products = count_of("PRODUCT");
        let assembly_usages = count_of("NEXT_ASSEMBLY_USAGE_OCCURRENCE");

        let mut entity_types: Vec<EntityCount> = counts
            .into_iter()
            .map(|(name, count)| EntityCount { name, count })
            .collect();
        entity_types.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.name.cmp(&b.name)));

        Self {
            bytes: exchange.source().len(),
            header: read_header(exchange),
            sections: exchange.sections().len(),
            instances: exchange.instances().len(),
            complex_instances,
            products,
            assembly_usages,
            unresolved_references: graph.unresolved().len(),
            units,
            entity_types,
        }
    }
}

fn read_header(exchange: &Exchange<'_>) -> Header {
    let mut header = Header::default();
    if let Some(description) = exchange.header_entity("FILE_DESCRIPTION") {
        header.description = texts(description.param(0));
        header.implementation_level = text(description.param(1).as_ref());
    }
    if let Some(name) = exchange.header_entity("FILE_NAME") {
        header.name = text(name.param(0).as_ref());
        header.time_stamp = text(name.param(1).as_ref());
        header.author = texts(name.param(2));
        header.organization = texts(name.param(3));
        header.preprocessor_version = text(name.param(4).as_ref());
        header.originating_system = text(name.param(5).as_ref());
        header.authorization = text(name.param(6).as_ref());
    }
    if let Some(schema) = exchange.header_entity("FILE_SCHEMA") {
        header.schemas = texts(schema.param(0));
    }
    header
}

/// Records the unit an instance declares, if it declares one.
fn collect_unit(units: &mut Units, exchange: &Exchange<'_>, instance: &Instance) {
    let records = exchange.records(instance);
    if !records
        .clone()
        .any(|r| ends_with_ignore_case(r.name(), b"_UNIT"))
    {
        return;
    }
    let has = |name: &str| records.clone().any(|r| r.is(name));
    let list = if has("LENGTH_UNIT") {
        &mut units.length
    } else if has("PLANE_ANGLE_UNIT") {
        &mut units.plane_angle
    } else if has("SOLID_ANGLE_UNIT") {
        &mut units.solid_angle
    } else {
        return;
    };
    let label = records
        .clone()
        .find(|r| r.is("SI_UNIT"))
        .and_then(si_label)
        .or_else(|| {
            records
                .clone()
                .find(|r| r.is("CONVERSION_BASED_UNIT"))
                .and_then(|r| text(r.param(0).as_ref()))
                .map(|name| name.to_lowercase())
        });
    if let Some(label) = label {
        if !list.contains(&label) {
            list.push(label);
        }
    }
}

/// `millimetre` for `SI_UNIT(.MILLI.,.METRE.)`, `radian` for
/// `SI_UNIT($,.RADIAN.)`.
fn si_label(record: Record<'_>) -> Option<String> {
    let parts: Vec<&str> = record
        .params()
        .filter_map(|param| param.literal()?.enumeration())
        .collect();
    (!parts.is_empty()).then(|| parts.concat().to_ascii_lowercase())
}

fn ends_with_ignore_case(name: &[u8], suffix: &[u8]) -> bool {
    name.len() >= suffix.len() && name[name.len() - suffix.len()..].eq_ignore_ascii_case(suffix)
}

/// A string parameter's decoded value; `None` if absent, not a string, or
/// blank. A string with a malformed escape falls back to its raw text.
fn text(param: Option<&Param<'_>>) -> Option<String> {
    let Some(Param::String(literal)) = param else {
        return None;
    };
    let value = literal.decode().map_or_else(
        |_| String::from_utf8_lossy(literal.text()).into_owned(),
        Cow::into_owned,
    );
    (!value.trim().is_empty()).then_some(value)
}

/// The non-blank strings of a list parameter.
fn texts(param: Option<Param<'_>>) -> Vec<String> {
    param
        .and_then(|param| param.list())
        .map(|items| items.filter_map(|item| text(Some(&item))).collect())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::p21::parse;

    const SAMPLE: &str = "ISO-10303-21;
HEADER;
FILE_DESCRIPTION(('bracket assembly',''),'2;1');
FILE_NAME('bracket.stp','2026-09-12T10:00:00',('J. Doe'),(''),'ST-DEVELOPER','CAD 1.0','');
FILE_SCHEMA(('AP242_MANAGED_MODEL_BASED_3D_ENGINEERING_MIM_LF { 1 0 10303 442 1 1 4 }'));
ENDSEC;
DATA;
#1=PRODUCT('a','assembly','',(#9));
#2=PRODUCT('b','bracket','',(#9));
#3=product('c','bolt','',(#9));
#4=NEXT_ASSEMBLY_USAGE_OCCURRENCE('1','','',#5,#6,$);
#5=PRODUCT_DEFINITION('','',#1,#9);
#6=PRODUCT_DEFINITION('','',#2,#9);
#7=(LENGTH_UNIT()NAMED_UNIT(*)SI_UNIT(.MILLI.,.METRE.));
#8=(NAMED_UNIT(*)PLANE_ANGLE_UNIT()SI_UNIT($,.RADIAN.));
#10=(CONVERSION_BASED_UNIT('DEGREE',#11)NAMED_UNIT(#12)PLANE_ANGLE_UNIT());
#11=PLANE_ANGLE_MEASURE_WITH_UNIT(PLANE_ANGLE_MEASURE(0.0174532925),#8);
#12=DIMENSIONAL_EXPONENTS(0.,0.,0.,0.,0.,0.,0.);
#13=(NAMED_UNIT(*)SI_UNIT($,.STERADIAN.)SOLID_ANGLE_UNIT());
#14=(LENGTH_UNIT()NAMED_UNIT(*)SI_UNIT(.MILLI.,.METRE.));
ENDSEC;
END-ISO-10303-21;
";

    fn info() -> Info {
        Info::new(&Graph::build(parse(SAMPLE.as_bytes()).unwrap()))
    }

    #[test]
    fn header_fields_are_decoded() {
        let header = info().header;
        assert_eq!(header.description, ["bracket assembly"]);
        assert_eq!(header.implementation_level.as_deref(), Some("2;1"));
        assert_eq!(header.name.as_deref(), Some("bracket.stp"));
        assert_eq!(header.time_stamp.as_deref(), Some("2026-09-12T10:00:00"));
        assert_eq!(header.author, ["J. Doe"]);
        assert!(header.organization.is_empty());
        assert_eq!(header.preprocessor_version.as_deref(), Some("ST-DEVELOPER"));
        assert_eq!(header.originating_system.as_deref(), Some("CAD 1.0"));
        assert_eq!(header.authorization, None);
        assert_eq!(
            header.schemas,
            ["AP242_MANAGED_MODEL_BASED_3D_ENGINEERING_MIM_LF { 1 0 10303 442 1 1 4 }"]
        );
    }

    #[test]
    fn counts() {
        let info = info();
        assert_eq!(info.bytes, SAMPLE.len());
        assert_eq!(info.sections, 1);
        assert_eq!(info.instances, 13);
        assert_eq!(info.complex_instances, 5);
        assert_eq!(info.products, 3, "PRODUCT and product are the same type");
        assert_eq!(info.assembly_usages, 1);
        assert_eq!(info.unresolved_references, 5, "#9 is referenced five times");
    }

    #[test]
    fn units_are_listed_once() {
        let units = info().units;
        assert_eq!(units.length, ["millimetre"]);
        assert_eq!(units.plane_angle, ["radian", "degree"]);
        assert_eq!(units.solid_angle, ["steradian"]);
    }

    #[test]
    fn entity_types_are_sorted_by_count_then_name() {
        let types = info().entity_types;
        let top: Vec<(&str, usize)> = types
            .iter()
            .take(4)
            .map(|t| (t.name.as_str(), t.count))
            .collect();
        assert_eq!(
            top,
            [
                ("NAMED_UNIT", 5),
                ("SI_UNIT", 4),
                ("PRODUCT", 3),
                ("LENGTH_UNIT", 2)
            ]
        );
        assert_eq!(types.len(), 11);
        assert!(
            types.iter().all(|t| t.name != "PLANE_ANGLE_MEASURE"),
            "typed parameters are not records"
        );
    }
}
