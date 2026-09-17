//! Semantic PMI: geometric tolerances, dimensions and datums.
//!
//! This is what `stepq pmi` prints. AP242 represents product and
//! manufacturing information twice: as presentation (annotation curves and
//! text for display) and as semantics (tolerances, datums and dimensions a
//! program can act on). This module reads the semantic half from the entity
//! graph. Every value is kept as written — the source text of a measure,
//! never re-printed — and every item is tied to the product definition it
//! belongs to.

use std::collections::HashSet;

use crate::model::{Graph, ProductStructure};
use crate::p21::{Exchange, Literal, Param, Record};
use crate::props::{Subject, Value, representation_items, subject, text, value};

/// Everything [`pmi`] found, each list in file order.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct Pmi {
    /// Every `datum`.
    pub datums: Vec<Datum>,
    /// Every geometric tolerance.
    pub tolerances: Vec<Tolerance>,
    /// Every size and location dimension.
    pub dimensions: Vec<Dimension>,
}

impl Pmi {
    /// True if the file holds no semantic PMI.
    pub fn is_empty(&self) -> bool {
        self.datums.is_empty() && self.tolerances.is_empty() && self.dimensions.is_empty()
    }
}

/// A datum, such as `A`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct Datum {
    /// The `#id` of the `datum`.
    pub instance: u64,
    /// `datum.identification`, the letter on the drawing.
    pub label: String,
    /// The datum itself, and the product definition it belongs to.
    pub subject: Subject,
}

/// A geometric tolerance, such as a position tolerance of 0.75 to A|B|C.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct Tolerance {
    /// The `#id` of the tolerance.
    pub instance: u64,
    /// The characteristic, lower case with spaces: `position`, `flatness`,
    /// `surface profile`, …; `geometric` if the file names none.
    pub kind: String,
    /// `geometric_tolerance.name`.
    pub name: Option<String>,
    /// `geometric_tolerance.magnitude`.
    pub magnitude: Option<Value>,
    /// The datum references, in order of precedence.
    pub datums: Vec<DatumReference>,
    /// Tolerance modifiers, lower case, such as
    /// `maximum_material_requirement`.
    pub modifiers: Vec<String>,
    /// For a tolerance per unit area or length, that unit.
    pub unit_size: Option<Value>,
    /// The toleranced shape aspect.
    pub target: Subject,
}

/// One datum in a tolerance's datum reference frame.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct DatumReference {
    /// The datum's label; a common datum's labels joined with `-`.
    pub label: String,
    /// Datum modifiers, lower case.
    pub modifiers: Vec<String>,
}

/// Whether a dimension is a size or a location.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "lowercase"))]
#[non_exhaustive]
pub enum DimensionKind {
    /// A `dimensional_size`: the size of one feature.
    Size,
    /// A `dimensional_location`: the distance or angle between two.
    Location,
}

/// A dimension with its values and tolerance.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct Dimension {
    /// The `#id` of the dimension.
    pub instance: u64,
    /// Size or location.
    pub kind: DimensionKind,
    /// The dimension's name, such as `diameter` or `linear distance`.
    pub name: Option<String>,
    /// The values its representation gives, such as `nominal value`.
    pub values: Vec<Value>,
    /// A plus-minus tolerance's bounds, if it has one.
    pub tolerance: Option<Bounds>,
    /// The feature a size applies to, or the feature a location is measured
    /// from.
    pub target: Subject,
    /// For a location, the feature it is measured to.
    pub related: Option<Subject>,
}

/// The lower and upper bounds of a plus-minus tolerance.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct Bounds {
    /// `tolerance_value.lower_bound`.
    pub lower: Option<Value>,
    /// `tolerance_value.upper_bound`.
    pub upper: Option<Value>,
}

/// The characteristic subtypes of `geometric_tolerance`, which a simple
/// instance names directly.
const TOLERANCE_KINDS: &[&str] = &[
    "ANGULARITY_TOLERANCE",
    "CIRCULAR_RUNOUT_TOLERANCE",
    "COAXIALITY_TOLERANCE",
    "CONCENTRICITY_TOLERANCE",
    "CYLINDRICITY_TOLERANCE",
    "FLATNESS_TOLERANCE",
    "LINE_PROFILE_TOLERANCE",
    "PARALLELISM_TOLERANCE",
    "PERPENDICULARITY_TOLERANCE",
    "POSITION_TOLERANCE",
    "ROUNDNESS_TOLERANCE",
    "STRAIGHTNESS_TOLERANCE",
    "SURFACE_PROFILE_TOLERANCE",
    "SYMMETRY_TOLERANCE",
    "TOTAL_RUNOUT_TOLERANCE",
];

/// The subtypes of `geometric_tolerance_with_datum_reference`; a simple
/// instance of one keeps its datum system as attribute 4.
const WITH_DATUMS: &[&str] = &[
    "ANGULARITY_TOLERANCE",
    "CIRCULAR_RUNOUT_TOLERANCE",
    "COAXIALITY_TOLERANCE",
    "CONCENTRICITY_TOLERANCE",
    "PARALLELISM_TOLERANCE",
    "PERPENDICULARITY_TOLERANCE",
    "SYMMETRY_TOLERANCE",
    "TOTAL_RUNOUT_TOLERANCE",
];

const SIZES: &[&str] = &[
    "DIMENSIONAL_SIZE",
    "ANGULAR_SIZE",
    "DIMENSIONAL_SIZE_WITH_PATH",
];
const LOCATIONS: &[&str] = &[
    "DIMENSIONAL_LOCATION",
    "ANGULAR_LOCATION",
    "DIMENSIONAL_LOCATION_WITH_PATH",
];

/// Reads the semantic PMI of the file behind `graph`. `structure` must be
/// built from the same graph.
pub fn pmi(graph: &Graph<'_>, structure: &ProductStructure) -> Pmi {
    let exchange = graph.exchange();
    let definitions: HashSet<u64> = structure
        .definitions()
        .iter()
        .map(|definition| definition.instance)
        .collect();
    let mut found = Pmi {
        datums: Vec::new(),
        tolerances: Vec::new(),
        dimensions: Vec::new(),
    };

    for (node, instance) in exchange.instances().iter().enumerate() {
        let records: Vec<Record<'_>> = exchange.records(instance).collect();
        let find = |name: &str| records.iter().copied().find(|record| record.is(name));
        let simple = if instance.is_complex() {
            None
        } else {
            records.first().copied()
        };
        let simple_name = simple.map(|record| upper(record.name()));
        let simple_is = |names: &[&str]| simple_name.as_deref().is_some_and(|n| names.contains(&n));

        if let Some(record) = simple.filter(|r| r.is("DATUM")) {
            if let Some(label) = text(record.param(4)) {
                found.datums.push(Datum {
                    instance: instance.id,
                    label,
                    subject: subject(exchange, &definitions, instance.id),
                });
            }
        } else if let Some(base) =
            find("GEOMETRIC_TOLERANCE").or(simple.filter(|_| simple_is(TOLERANCE_KINDS)))
        {
            let datum_system = match (find("GEOMETRIC_TOLERANCE_WITH_DATUM_REFERENCE"), simple) {
                (Some(record), _) => record.param(0),
                (None, Some(record)) if simple_is(WITH_DATUMS) => record.param(4),
                _ => None,
            };
            let kind = records
                .iter()
                .map(|record| upper(record.name()))
                .find(|name| TOLERANCE_KINDS.contains(&name.as_str()))
                .map_or_else(
                    || "geometric".to_owned(),
                    |name| {
                        name.trim_end_matches("_TOLERANCE")
                            .to_ascii_lowercase()
                            .replace('_', " ")
                    },
                );
            let Some(target) = base.param(3).and_then(|p| p.reference()) else {
                continue;
            };
            found.tolerances.push(Tolerance {
                instance: instance.id,
                kind,
                name: text(base.param(0)),
                magnitude: base
                    .param(2)
                    .and_then(|p| p.reference())
                    .and_then(|id| value(exchange, id)),
                datums: datum_references(exchange, datum_system),
                modifiers: find("GEOMETRIC_TOLERANCE_WITH_MODIFIERS")
                    .map(|record| enumerations(record.param(0)))
                    .unwrap_or_default(),
                unit_size: find("GEOMETRIC_TOLERANCE_WITH_DEFINED_UNIT")
                    .and_then(|record| record.param(0))
                    .and_then(|p| p.reference())
                    .and_then(|id| value(exchange, id)),
                target: subject(exchange, &definitions, target),
            });
        } else if let Some(record) = simple.filter(|_| simple_is(SIZES) || simple_is(LOCATIONS)) {
            let size = simple_is(SIZES);
            let (name, target, related) = if size {
                (record.param(1), record.param(0), None)
            } else {
                (record.param(0), record.param(2), record.param(3))
            };
            let Some(target) = target.and_then(|p| p.reference()) else {
                continue;
            };
            let (values, tolerance) = dimension_values(graph, node, instance.id);
            found.dimensions.push(Dimension {
                instance: instance.id,
                kind: if size {
                    DimensionKind::Size
                } else {
                    DimensionKind::Location
                },
                name: text(name),
                values,
                tolerance,
                target: subject(exchange, &definitions, target),
                related: related
                    .and_then(|p| p.reference())
                    .map(|id| subject(exchange, &definitions, id)),
            });
        }
    }
    found
}

fn upper(name: &[u8]) -> String {
    String::from_utf8_lossy(name).to_ascii_uppercase()
}

/// The enumeration values in a set or list, lower case.
fn enumerations(param: Option<Param<'_>>) -> Vec<String> {
    param
        .and_then(|p| p.list())
        .into_iter()
        .flatten()
        .filter_map(|p| p.literal())
        .filter_map(Literal::enumeration)
        .map(str::to_ascii_lowercase)
        .collect()
}

/// A tolerance's datum references: through a `datum_system`'s compartments,
/// or older files' `datum_reference`s ordered by precedence.
fn datum_references(exchange: &Exchange<'_>, param: Option<Param<'_>>) -> Vec<DatumReference> {
    let ids: Vec<u64> = param
        .and_then(|p| p.list())
        .into_iter()
        .flatten()
        .filter_map(|p| p.reference())
        .collect();
    let mut references = Vec::new();
    let mut by_precedence = Vec::new();
    for id in ids {
        let Some(record) = exchange.get(id).and_then(|i| exchange.records(i).next()) else {
            continue;
        };
        if record.is("DATUM_SYSTEM") {
            let constituents = record.param(4).and_then(|p| p.list()).into_iter().flatten();
            for compartment in constituents.filter_map(|p| p.reference()) {
                if let Some(reference) = compartment_reference(exchange, compartment) {
                    references.push(reference);
                }
            }
        } else if record.is("DATUM_REFERENCE") {
            let precedence = record
                .param(0)
                .and_then(|p| p.literal())
                .and_then(Literal::to_i64)
                .unwrap_or(i64::MAX);
            if let Some(label) = record
                .param(1)
                .and_then(|p| p.reference())
                .and_then(|d| datum_label(exchange, d))
            {
                by_precedence.push((
                    precedence,
                    DatumReference {
                        label,
                        modifiers: Vec::new(),
                    },
                ));
            }
        } else if let Some(reference) = compartment_reference(exchange, id) {
            references.push(reference);
        }
    }
    by_precedence.sort_by_key(|(precedence, _)| *precedence);
    references.extend(by_precedence.into_iter().map(|(_, reference)| reference));
    references
}

/// A `datum_reference_compartment` or `_element`: its base datum (or common
/// datum) and modifiers.
fn compartment_reference(exchange: &Exchange<'_>, id: u64) -> Option<DatumReference> {
    let instance = exchange.get(id)?;
    let record = exchange.records(instance).next()?;
    if !upper(record.name()).starts_with("DATUM_REFERENCE_") {
        return None;
    }
    let base = record.param(4)?;
    let label = match base.list() {
        // A common datum: a list of datum reference elements.
        Some(elements) => elements
            .filter_map(|p| p.reference())
            .filter_map(|element| compartment_reference(exchange, element))
            .map(|element| element.label)
            .collect::<Vec<_>>()
            .join("-"),
        None => base.reference().and_then(|d| datum_label(exchange, d))?,
    };
    let modifiers = record
        .param(5)
        .map(|p| enumerations(Some(p)))
        .unwrap_or_default();
    Some(DatumReference { label, modifiers })
}

fn datum_label(exchange: &Exchange<'_>, id: u64) -> Option<String> {
    let record = exchange.get(id).and_then(|i| exchange.records(i).next())?;
    record.is("DATUM").then(|| text(record.param(4))).flatten()
}

/// A dimension's values, from the `shape_dimension_representation` its
/// `dimensional_characteristic_representation` names, and its plus-minus
/// tolerance, from a `plus_minus_tolerance` whose range is a
/// `tolerance_value`.
fn dimension_values(graph: &Graph<'_>, node: usize, id: u64) -> (Vec<Value>, Option<Bounds>) {
    let exchange = graph.exchange();
    let mut values = Vec::new();
    let mut tolerance = None;
    for &user in graph.referenced_by(node) {
        let Some(record) = exchange.records(graph.instance(user)).next() else {
            continue;
        };
        let points_at = |index| record.param(index).and_then(|p| p.reference()) == Some(id);
        if record.is("DIMENSIONAL_CHARACTERISTIC_REPRESENTATION") && points_at(0) {
            let items = record
                .param(1)
                .and_then(|p| p.reference())
                .map(|rep| representation_items(exchange, rep))
                .unwrap_or_default();
            values.extend(items.into_iter().filter_map(|item| value(exchange, item)));
        } else if record.is("PLUS_MINUS_TOLERANCE") && points_at(1) {
            let range = record
                .param(0)
                .and_then(|p| p.reference())
                .and_then(|range| exchange.get(range))
                .and_then(|range| exchange.records(range).next())
                .filter(|range| range.is("TOLERANCE_VALUE"));
            if let Some(range) = range {
                let bound = |index| {
                    range
                        .param(index)
                        .and_then(|p| p.reference())
                        .and_then(|b| value(exchange, b))
                };
                tolerance = Some(Bounds {
                    lower: bound(0),
                    upper: bound(1),
                });
            }
        }
    }
    (values, tolerance)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::p21::parse;

    /// Datums A and B (B with a modifier), a position tolerance to them and
    /// a flatness tolerance on a hole, a toleranced diameter and a location.
    pub(crate) const SAMPLE: &str = "ISO-10303-21;HEADER;FILE_SCHEMA(('AP242'));ENDSEC;DATA;
#1=APPLICATION_CONTEXT('');
#2=PRODUCT_DEFINITION_CONTEXT('',#1,'design');
#10=PRODUCT('P','part','',());
#11=PRODUCT_DEFINITION_FORMATION('','',#10);
#12=PRODUCT_DEFINITION('design','',#11,#2);
#20=PRODUCT_DEFINITION_SHAPE('','',#12);
#30=DATUM('',$,#20,.F.,'A');
#31=DATUM('',$,#20,.F.,'B');
#40=DATUM_REFERENCE_COMPARTMENT('',$,#20,.F.,#30,$);
#41=DATUM_REFERENCE_COMPARTMENT('',$,#20,.F.,#31,(.MAXIMUM_MATERIAL_REQUIREMENT.));
#50=DATUM_SYSTEM('DS',$,#20,.F.,(#40,#41));
#60=SHAPE_ASPECT('','hole',#20,.F.);
#61=SHAPE_ASPECT('','face',#20,.F.);
#70=(LENGTH_MEASURE_WITH_UNIT()MEASURE_REPRESENTATION_ITEM()MEASURE_WITH_UNIT(LENGTH_MEASURE(0.75),#99)REPRESENTATION_ITEM(''));
#80=(GEOMETRIC_TOLERANCE('Position.1','',#70,#60)GEOMETRIC_TOLERANCE_WITH_DATUM_REFERENCE((#50))GEOMETRIC_TOLERANCE_WITH_MODIFIERS((.MAXIMUM_MATERIAL_REQUIREMENT.))POSITION_TOLERANCE());
#81=FLATNESS_TOLERANCE('Flatness.1','',#82,#61);
#82=LENGTH_MEASURE_WITH_UNIT(LENGTH_MEASURE(0.05),#99);
#90=DIMENSIONAL_SIZE(#60,'diameter');
#91=DIMENSIONAL_CHARACTERISTIC_REPRESENTATION(#90,#92);
#92=SHAPE_DIMENSION_REPRESENTATION('',(#93),#1);
#93=(LENGTH_MEASURE_WITH_UNIT()MEASURE_REPRESENTATION_ITEM()MEASURE_WITH_UNIT(LENGTH_MEASURE(35.),#99)REPRESENTATION_ITEM('nominal value'));
#94=PLUS_MINUS_TOLERANCE(#95,#90);
#95=TOLERANCE_VALUE(#96,#97);
#96=(LENGTH_MEASURE_WITH_UNIT()MEASURE_REPRESENTATION_ITEM()MEASURE_WITH_UNIT(LENGTH_MEASURE(-0.2),#99)REPRESENTATION_ITEM(''));
#97=(LENGTH_MEASURE_WITH_UNIT()MEASURE_REPRESENTATION_ITEM()MEASURE_WITH_UNIT(LENGTH_MEASURE(0.),#99)REPRESENTATION_ITEM(''));
#98=DIMENSIONAL_LOCATION('linear distance',$,#60,#61);
#99=(LENGTH_UNIT()NAMED_UNIT(*)SI_UNIT(.MILLI.,.METRE.));
ENDSEC;END-ISO-10303-21;";

    fn found() -> Pmi {
        let graph = Graph::new(parse(SAMPLE.as_bytes()).unwrap()).unwrap();
        let structure = ProductStructure::new(&graph);
        pmi(&graph, &structure)
    }

    fn on(instance: u64, entity: &str) -> Subject {
        Subject {
            instance,
            entity: entity.to_owned(),
            product_definition: Some(12),
        }
    }

    fn length(instance: u64, name: Option<&str>, value: &str) -> Value {
        Value {
            instance,
            name: name.map(str::to_owned),
            measure: Some("LENGTH_MEASURE".to_owned()),
            value: value.to_owned(),
            unit: Some(99),
        }
    }

    #[test]
    fn datums_are_read_by_letter() {
        let labels: Vec<(u64, String)> = found()
            .datums
            .into_iter()
            .map(|datum| (datum.subject.product_definition.unwrap(), datum.label))
            .collect();
        assert_eq!(labels, [(12, "A".to_owned()), (12, "B".to_owned())]);
    }

    #[test]
    fn tolerances_have_their_characteristic_magnitude_and_datums() {
        let found = found();
        assert_eq!(
            found.tolerances[0],
            Tolerance {
                instance: 80,
                kind: "position".to_owned(),
                name: Some("Position.1".to_owned()),
                magnitude: Some(length(70, Some(""), "0.75")),
                datums: vec![
                    DatumReference {
                        label: "A".to_owned(),
                        modifiers: vec![],
                    },
                    DatumReference {
                        label: "B".to_owned(),
                        modifiers: vec!["maximum_material_requirement".to_owned()],
                    },
                ],
                modifiers: vec!["maximum_material_requirement".to_owned()],
                unit_size: None,
                target: on(60, "SHAPE_ASPECT"),
            }
        );
        let flatness = &found.tolerances[1];
        assert_eq!(flatness.kind, "flatness");
        assert_eq!(flatness.magnitude, Some(length(82, None, "0.05")));
        assert!(flatness.datums.is_empty());
        assert_eq!(flatness.target, on(61, "SHAPE_ASPECT"));
    }

    #[test]
    fn dimensions_have_values_tolerances_and_features() {
        let found = found();
        assert_eq!(
            found.dimensions,
            [
                Dimension {
                    instance: 90,
                    kind: DimensionKind::Size,
                    name: Some("diameter".to_owned()),
                    values: vec![length(93, Some("nominal value"), "35.")],
                    tolerance: Some(Bounds {
                        lower: Some(length(96, Some(""), "-0.2")),
                        upper: Some(length(97, Some(""), "0.")),
                    }),
                    target: on(60, "SHAPE_ASPECT"),
                    related: None,
                },
                Dimension {
                    instance: 98,
                    kind: DimensionKind::Location,
                    name: Some("linear distance".to_owned()),
                    values: vec![],
                    tolerance: None,
                    target: on(60, "SHAPE_ASPECT"),
                    related: Some(on(61, "SHAPE_ASPECT")),
                },
            ]
        );
        assert!(!found.is_empty());
    }
}
