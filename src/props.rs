//! Product properties: user-defined attributes, validation properties and
//! persistent identifiers.
//!
//! This is what `stepq props` prints. A property is a `property_definition`
//! that points at what it describes: a product definition, its shape, or a
//! shape aspect of it. Its values are the items of the representations that
//! `property_definition_representation`s tie to it. Values are returned as
//! written: numbers are the source text, never re-parsed and re-printed.

use std::borrow::Cow;
use std::collections::HashSet;

use crate::model::{Graph, ProductStructure};
use crate::p21::{Exchange, Instance, Literal, Param, Record};

/// Everything [`properties`] found.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct Properties {
    /// Every `property_definition`, in file order.
    pub properties: Vec<Property>,
    /// Every `id_attribute`, in file order.
    pub identifiers: Vec<Identifier>,
}

/// One `property_definition` and its values.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct Property {
    /// The `#id` of the `property_definition`.
    pub instance: u64,
    /// What kind of property this is.
    pub kind: PropertyKind,
    /// `property_definition.name`.
    pub name: Option<String>,
    /// `property_definition.description`.
    pub description: Option<String>,
    /// What the property describes.
    pub subject: Subject,
    /// Values from every representation tied to the property, in order.
    pub values: Vec<Value>,
}

impl Property {
    /// A name for display: the description of a validation property
    /// (`volume of #602`), otherwise the name, falling back to the
    /// description.
    pub fn label(&self) -> &str {
        let name = self.name.as_deref().filter(|name| !name.is_empty());
        let description = self.description.as_deref().filter(|d| !d.is_empty());
        let label = if self.kind == PropertyKind::Validation {
            description.or(name)
        } else {
            name.or(description)
        };
        label.unwrap_or_default()
    }
}

/// The kind of a property.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
#[non_exhaustive]
pub enum PropertyKind {
    /// A validation property: geometric (volume, area, centroid, …) or of
    /// attributes (how many user attributes of each type), named
    /// `geometric validation property` or `attribute validation property`,
    /// with spaces or underscores.
    Validation,
    /// A user-defined attribute: associated with a `general_property`, or
    /// described as `user defined attribute`. Serialized as `user`, the
    /// name `stepq props --kind` uses.
    #[cfg_attr(feature = "serde", serde(rename = "user"))]
    UserDefined,
    /// Anything else.
    Other,
}

/// What a property or identifier is attached to.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct Subject {
    /// The `#id` the property or identifier points at.
    pub instance: u64,
    /// Its upper-cased entity type; for a complex instance, the partial
    /// entities joined with `+`.
    pub entity: String,
    /// The product definition it belongs to, found by following product
    /// definition shapes, shape aspects and property definitions; `None` if
    /// that leads nowhere, as for a property of an assembly usage.
    pub product_definition: Option<u64>,
}

/// One value of a property: one representation item.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct Value {
    /// The `#id` of the representation item.
    pub instance: u64,
    /// The item's name.
    pub name: Option<String>,
    /// The measure type, such as `VOLUME_MEASURE`, for measure and value
    /// items; the entity type for other items.
    pub measure: Option<String>,
    /// The value as written: a number's source text, a decoded string, or
    /// for items that are not values (a centroid point, say) the item's
    /// whole instance text.
    pub value: String,
    /// The `#id` of the unit, for measures with one.
    pub unit: Option<u64>,
}

/// One `id_attribute`: a persistent identifier.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct Identifier {
    /// The `#id` of the `id_attribute`.
    pub instance: u64,
    /// The identifier.
    pub id: String,
    /// What it identifies.
    pub subject: Subject,
}

/// Finds every property and identifier in the file behind `graph`.
/// `structure` must be built from the same graph; it says which instances
/// are product definitions.
pub fn properties(graph: &Graph<'_>, structure: &ProductStructure) -> Properties {
    let exchange = graph.exchange();
    let definitions: HashSet<u64> = structure
        .definitions()
        .iter()
        .map(|definition| definition.instance)
        .collect();
    let mut properties = Vec::new();
    let mut identifiers = Vec::new();

    for (node, instance) in exchange.instances().iter().enumerate() {
        let Some(record) = exchange.records(instance).next() else {
            continue;
        };
        if instance.is_complex() {
            continue;
        }
        if record.is("PROPERTY_DEFINITION") {
            let Some(target) = record.param(2).and_then(|p| p.reference()) else {
                continue;
            };
            let name = text(record.param(0));
            let description = text(record.param(1));
            let mut values = Vec::new();
            let mut general = false;
            for &user in graph.referenced_by(node) {
                let user = graph.instance(user);
                let Some(link) = exchange.records(user).next() else {
                    continue;
                };
                let points_here =
                    |index| link.param(index).and_then(|p| p.reference()) == Some(instance.id);
                if (link.is("PROPERTY_DEFINITION_REPRESENTATION")
                    || link.is("SHAPE_DEFINITION_REPRESENTATION"))
                    && points_here(0)
                {
                    let items = link
                        .param(1)
                        .and_then(|p| p.reference())
                        .map(|rep| representation_items(exchange, rep))
                        .unwrap_or_default();
                    values.extend(items.into_iter().filter_map(|item| value(exchange, item)));
                } else if link.is("GENERAL_PROPERTY_ASSOCIATION") && points_here(3) {
                    general = true;
                }
            }
            // AP214 practice writes `geometric_validation_property`, AP242
            // practice `geometric validation property`.
            let is = |field: &Option<String>, expected: &str| {
                field
                    .as_deref()
                    .is_some_and(|field| field.replace('_', " ").eq_ignore_ascii_case(expected))
            };
            let kind = if is(&name, "geometric validation property")
                || is(&name, "attribute validation property")
            {
                PropertyKind::Validation
            } else if general || is(&description, "user defined attribute") {
                PropertyKind::UserDefined
            } else {
                PropertyKind::Other
            };
            properties.push(Property {
                instance: instance.id,
                kind,
                name,
                description,
                subject: subject(exchange, &definitions, target),
                values,
            });
        } else if record.is("ID_ATTRIBUTE") {
            let (Some(id), Some(target)) = (
                text(record.param(0)),
                record.param(1).and_then(|p| p.reference()),
            ) else {
                continue;
            };
            identifiers.push(Identifier {
                instance: instance.id,
                id,
                subject: subject(exchange, &definitions, target),
            });
        }
    }
    Properties {
        properties,
        identifiers,
    }
}

/// The entities, upper-cased and joined with `+`, of instance `#id`.
pub(crate) fn entity(exchange: &Exchange<'_>, instance: &Instance) -> String {
    exchange
        .records(instance)
        .map(|record| upper(record.name()))
        .collect::<Vec<_>>()
        .join("+")
}

fn upper(name: &[u8]) -> String {
    String::from_utf8_lossy(name).to_ascii_uppercase()
}

/// What `#target` is and which of `definitions` it belongs to.
pub(crate) fn subject(exchange: &Exchange<'_>, definitions: &HashSet<u64>, target: u64) -> Subject {
    let entity = exchange
        .get(target)
        .map(|instance| entity(exchange, instance))
        .unwrap_or_default();
    // Shapes, shape aspects (and their subtypes, such as datums) and
    // property definitions all keep what they describe as attribute 2.
    let mut current = target;
    let mut product_definition = None;
    for _ in 0..16 {
        if definitions.contains(&current) {
            product_definition = Some(current);
            break;
        }
        let Some(instance) = exchange.get(current) else {
            break;
        };
        let record = exchange.records(instance).find(|record| {
            let name = upper(record.name());
            name == "PRODUCT_DEFINITION_SHAPE"
                || name == "PROPERTY_DEFINITION"
                || name.contains("SHAPE_ASPECT")
                || name.starts_with("DATUM")
        });
        let Some(next) = record.and_then(|r| r.param(2)).and_then(|p| p.reference()) else {
            break;
        };
        current = next;
    }
    Subject {
        instance: target,
        entity,
        product_definition,
    }
}

/// The items of representation `#id`, in order.
pub(crate) fn representation_items(exchange: &Exchange<'_>, id: u64) -> Vec<u64> {
    let Some(instance) = exchange.get(id) else {
        return Vec::new();
    };
    let mut records = exchange.records(instance);
    let record = if instance.is_complex() {
        records.find(|r| r.is("REPRESENTATION"))
    } else {
        records.next()
    };
    record
        .and_then(|r| r.param(1))
        .and_then(|p| p.list())
        .into_iter()
        .flatten()
        .filter_map(|p| p.reference())
        .collect()
}

/// Instance `#id` read as a value: a measure, a descriptive or literal item,
/// or failing those its whole text.
pub(crate) fn value(exchange: &Exchange<'_>, id: u64) -> Option<Value> {
    let instance = exchange.get(id)?;
    let records: Vec<Record<'_>> = exchange.records(instance).collect();
    let find = |name: &str| records.iter().copied().find(|r| r.is(name));
    let reference = |param: Option<Param<'_>>| param.and_then(|p| p.reference());

    if instance.is_complex() {
        if let (Some(item), Some(measure)) =
            (find("REPRESENTATION_ITEM"), find("MEASURE_WITH_UNIT"))
        {
            let (kind, value) = typed(measure.param(0));
            return Some(Value {
                instance: id,
                name: text(item.param(0)),
                measure: kind,
                value,
                unit: reference(measure.param(1)),
            });
        }
    } else if let Some(record) = records.first().copied() {
        if record.is("MEASURE_REPRESENTATION_ITEM") || record.is("VALUE_REPRESENTATION_ITEM") {
            let (kind, value) = typed(record.param(1));
            return Some(Value {
                instance: id,
                name: text(record.param(0)),
                measure: kind,
                value,
                unit: reference(record.param(2)),
            });
        }
        // LENGTH_MEASURE_WITH_UNIT(LENGTH_MEASURE(0.05),#99) and the like.
        if upper(record.name()).ends_with("MEASURE_WITH_UNIT") && record.params().count() == 2 {
            let (kind, value) = typed(record.param(0));
            return Some(Value {
                instance: id,
                name: None,
                measure: kind,
                value,
                unit: reference(record.param(1)),
            });
        }
        if record.is("DESCRIPTIVE_REPRESENTATION_ITEM") {
            return Some(Value {
                instance: id,
                name: text(record.param(0)),
                measure: None,
                value: text(record.param(1)).unwrap_or_default(),
                unit: None,
            });
        }
        // INTEGER_, REAL_, BOOLEAN_, STRING_REPRESENTATION_ITEM and the
        // like: a name and a plain literal.
        let literal = record.param(1).and_then(|p| p.literal());
        if upper(record.name()).ends_with("_REPRESENTATION_ITEM") && record.params().count() == 2 {
            if let Some(literal) = literal {
                return Some(Value {
                    instance: id,
                    name: text(record.param(0)),
                    measure: None,
                    value: literal_text(literal),
                    unit: None,
                });
            }
        }
    }
    // Not a value item: a centroid point, a placement, … Keep it whole.
    Some(Value {
        instance: id,
        name: records.first().and_then(|r| text(r.param(0))),
        measure: Some(entity(exchange, instance)),
        value: String::from_utf8_lossy(exchange.text(instance)).into_owned(),
        unit: None,
    })
}

/// A typed value such as `VOLUME_MEASURE(96858.9)`: its type and its
/// value's source text.
fn typed(param: Option<Param<'_>>) -> (Option<String>, String) {
    let Some(param) = param else {
        return (None, String::new());
    };
    match param.typed() {
        Some(record) => (
            Some(upper(record.name())),
            record
                .param(0)
                .and_then(|p| p.literal())
                .map(literal_text)
                .unwrap_or_default(),
        ),
        None => (None, param.literal().map(literal_text).unwrap_or_default()),
    }
}

/// A string literal decoded, or any other literal's source text.
fn literal_text(literal: Literal<'_>) -> String {
    let source = literal.text();
    if source.first() == Some(&b'\'') {
        if let Ok(decoded) = literal.decode() {
            return decoded.into_owned();
        }
    }
    String::from_utf8_lossy(source).into_owned()
}

/// A string attribute, decoded; `None` if unset or not a string.
pub(crate) fn text(param: Option<Param<'_>>) -> Option<String> {
    let literal = param?.literal()?;
    if literal.text().first() != Some(&b'\'') {
        return None;
    }
    literal.decode().ok().map(Cow::into_owned)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::p21::parse;

    /// A part with a validated volume on a shape aspect, a user-defined
    /// attribute with three values, a property of nothing product-shaped,
    /// and a persistent identifier.
    pub(crate) const SAMPLE: &str = "ISO-10303-21;HEADER;FILE_SCHEMA(('AP242'));ENDSEC;DATA;
#1=APPLICATION_CONTEXT('');
#10=PRODUCT('P1','bracket','',());
#11=PRODUCT_DEFINITION_FORMATION('','',#10);
#12=PRODUCT_DEFINITION('design','',#11,#13);
#13=PRODUCT_DEFINITION_CONTEXT('',#1,'design');
#20=PRODUCT_DEFINITION_SHAPE('','',#12);
#21=SHAPE_ASPECT('','solid',#20,.F.);
#30=PROPERTY_DEFINITION('geometric_validation_property','volume of solid',#21);
#31=PROPERTY_DEFINITION_REPRESENTATION(#30,#32);
#32=REPRESENTATION('volume',(#33),#1);
#33=MEASURE_REPRESENTATION_ITEM('volume measure',VOLUME_MEASURE(96858.91343205),#99);
#40=PROPERTY_DEFINITION('MATERIAL','user defined attribute',#12);
#41=GENERAL_PROPERTY('','MATERIAL','user defined attribute');
#42=GENERAL_PROPERTY_ASSOCIATION('user defined attribute','',#41,#40);
#43=PROPERTY_DEFINITION_REPRESENTATION(#40,#44);
#44=REPRESENTATION('',(#45,#46,#47),#1);
#45=DESCRIPTIVE_REPRESENTATION_ITEM('MATERIAL','Steel ''S235''');
#46=VALUE_REPRESENTATION_ITEM('THICKNESS',LENGTH_MEASURE(1.E0));
#47=(MEASURE_REPRESENTATION_ITEM()MEASURE_WITH_UNIT(MASS_MEASURE(2.5),#99)REPRESENTATION_ITEM('mass'));
#50=PROPERTY_DEFINITION('centroid','',#1);
#51=PROPERTY_DEFINITION_REPRESENTATION(#50,#52);
#52=REPRESENTATION('',(#53),#1);
#53=CARTESIAN_POINT('centre',(1.,2.,3.));
#70=ID_ATTRIBUTE('aspect.1',#21);
#99=DERIVED_UNIT(());
ENDSEC;END-ISO-10303-21;";

    #[test]
    fn ap242_spelling_and_literal_items() {
        let src = "ISO-10303-21;HEADER;FILE_SCHEMA(('AP242'));ENDSEC;DATA;
#1=APPLICATION_CONTEXT('');
#10=PRODUCT('P1','bracket','',());
#11=PRODUCT_DEFINITION_FORMATION('','',#10);
#12=PRODUCT_DEFINITION('design','',#11,#13);
#13=PRODUCT_DEFINITION_CONTEXT('',#1,'design');
#30=PROPERTY_DEFINITION('attribute validation property','',#12);
#31=PROPERTY_DEFINITION_REPRESENTATION(#30,#32);
#32=REPRESENTATION('',(#33,#34),#1);
#33=INTEGER_REPRESENTATION_ITEM('text user attributes',3.);
#34=STRING_REPRESENTATION_ITEM('note','it''s');
#40=PROPERTY_DEFINITION('Geometric Validation Property','volume',#12);
ENDSEC;END-ISO-10303-21;";
        let graph = Graph::new(parse(src.as_bytes()).unwrap()).unwrap();
        let found = properties(&graph, &ProductStructure::new(&graph));
        assert_eq!(found.properties[0].kind, PropertyKind::Validation);
        assert_eq!(found.properties[1].kind, PropertyKind::Validation);
        assert_eq!(
            found.properties[0].values,
            [
                Value {
                    instance: 33,
                    name: Some("text user attributes".to_owned()),
                    measure: None,
                    value: "3.".to_owned(),
                    unit: None,
                },
                Value {
                    instance: 34,
                    name: Some("note".to_owned()),
                    measure: None,
                    value: "it's".to_owned(),
                    unit: None,
                },
            ]
        );
    }

    fn found() -> Properties {
        let graph = Graph::new(parse(SAMPLE.as_bytes()).unwrap()).unwrap();
        let structure = ProductStructure::new(&graph);
        properties(&graph, &structure)
    }

    fn value(
        instance: u64,
        name: &str,
        measure: Option<&str>,
        value: &str,
        unit: Option<u64>,
    ) -> Value {
        Value {
            instance,
            name: Some(name.to_owned()),
            measure: measure.map(str::to_owned),
            value: value.to_owned(),
            unit,
        }
    }

    #[test]
    fn validation_properties_belong_to_the_part_through_their_shape_aspect() {
        let found = found();
        let volume = &found.properties[0];
        assert_eq!(volume.kind, PropertyKind::Validation);
        assert_eq!(volume.label(), "volume of solid");
        assert_eq!(
            volume.subject,
            Subject {
                instance: 21,
                entity: "SHAPE_ASPECT".to_owned(),
                product_definition: Some(12),
            }
        );
        assert_eq!(
            volume.values,
            [value(
                33,
                "volume measure",
                Some("VOLUME_MEASURE"),
                "96858.91343205",
                Some(99)
            )]
        );
    }

    #[test]
    fn user_defined_attributes_keep_values_as_written() {
        let found = found();
        let material = &found.properties[1];
        assert_eq!(material.kind, PropertyKind::UserDefined);
        assert_eq!(material.label(), "MATERIAL");
        assert_eq!(material.subject.product_definition, Some(12));
        assert_eq!(
            material.values,
            [
                value(45, "MATERIAL", None, "Steel 'S235'", None),
                value(46, "THICKNESS", Some("LENGTH_MEASURE"), "1.E0", None),
                value(47, "mass", Some("MASS_MEASURE"), "2.5", Some(99)),
            ]
        );
    }

    #[test]
    fn other_items_and_identifiers() {
        let found = found();
        let centroid = &found.properties[2];
        assert_eq!(centroid.kind, PropertyKind::Other);
        assert_eq!(centroid.subject.product_definition, None);
        assert_eq!(centroid.subject.entity, "APPLICATION_CONTEXT");
        assert_eq!(
            centroid.values,
            [value(
                53,
                "centre",
                Some("CARTESIAN_POINT"),
                "#53=CARTESIAN_POINT('centre',(1.,2.,3.));",
                None
            )]
        );
        assert_eq!(found.properties.len(), 3);
        assert_eq!(
            found.identifiers,
            [Identifier {
                instance: 70,
                id: "aspect.1".to_owned(),
                subject: Subject {
                    instance: 21,
                    entity: "SHAPE_ASPECT".to_owned(),
                    product_definition: Some(12),
                },
            }]
        );
    }
}
