//! A structural comparison of two STEP files.
//!
//! This is what `stepq diff` prints. Instance names mean nothing across two
//! exports of the same model, so files are compared by what they describe:
//! header fields and units, how many instances of each entity type they
//! hold, products (matched by `product.id`), how many of each component
//! every assembly uses, and property values. Geometry is compared only
//! through those counts and through validation properties, never
//! evaluated.

use std::collections::{BTreeMap, BTreeSet};

use crate::info::Info;
use crate::model::{Definition, Graph, ProductStructure};
use crate::props::properties;

/// Everything that differs between two files; see [`diff`].
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct Diff {
    /// Header fields, units and the instance count, by field name.
    pub header: Vec<FieldChange>,
    /// Entity types whose instance count differs, by name.
    pub entity_types: Vec<CountChange>,
    /// Products added, removed or changed, by key.
    pub products: Vec<ProductChange>,
    /// Assembly components whose total quantity differs, by parent and
    /// child key.
    pub components: Vec<ComponentChange>,
    /// Property values added, removed or changed, by product and property.
    pub properties: Vec<PropertyChange>,
}

impl Diff {
    /// True if nothing differs.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The number of differences in all sections.
    pub fn len(&self) -> usize {
        self.header.len()
            + self.entity_types.len()
            + self.products.len()
            + self.components.len()
            + self.properties.len()
    }
}

/// A named value in both files; `None` where the file has none.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct FieldChange {
    /// What differs, such as `originating_system`.
    pub field: String,
    /// The value in the first file.
    pub old: Option<String>,
    /// The value in the second file.
    pub new: Option<String>,
}

/// How many instances of an entity type each file has.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct CountChange {
    /// Upper-cased entity type.
    pub entity_type: String,
    /// Records in the first file.
    pub old: usize,
    /// Records in the second file.
    pub new: usize,
}

/// Whether something is only in the second file, only in the first, or in
/// both but different.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "lowercase"))]
pub enum ChangeKind {
    /// Only in the second file.
    Added,
    /// Only in the first file.
    Removed,
    /// In both, with different values.
    Changed,
}

/// A product definition that differs.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct ProductChange {
    /// The product's key: `product.id`, else its name, else the definition's
    /// `#id`; a repeated key gets ` (2)`, ` (3)`, … in file order.
    pub product: String,
    /// Added, removed or changed.
    pub kind: ChangeKind,
    /// For a changed product, the fields that differ.
    pub fields: Vec<FieldChange>,
}

/// The total quantity of one component in one assembly.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct ComponentChange {
    /// The assembly's product key.
    pub parent: String,
    /// The component's product key.
    pub child: String,
    /// Quantity in the first file; 0 if not used there.
    pub old: f64,
    /// Quantity in the second file; 0 if not used there.
    pub new: f64,
}

/// One property value that differs.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct PropertyChange {
    /// The product key, or empty for properties of no product definition.
    pub product: String,
    /// The property label and value name, with `#id`s replaced by `#`; a
    /// repeated key gets ` (2)`, ` (3)`, … in file order.
    pub property: String,
    /// Added, removed or changed.
    pub kind: ChangeKind,
    /// The value in the first file.
    pub old: Option<String>,
    /// The value in the second file.
    pub new: Option<String>,
}

/// Compares the files behind `old` and `new`.
pub fn diff(old: &Graph<'_>, new: &Graph<'_>) -> Diff {
    let (old_info, new_info) = (Info::new(old), Info::new(new));
    let (old_structure, new_structure) = (ProductStructure::new(old), ProductStructure::new(new));
    Diff {
        header: fields(&header_fields(&old_info), &header_fields(&new_info)),
        entity_types: entity_types(&old_info, &new_info),
        products: products(&old_structure, &new_structure),
        components: components(&old_structure, &new_structure),
        properties: property_changes(
            &property_values(old, &old_structure),
            &property_values(new, &new_structure),
        ),
    }
}

fn header_fields(info: &Info) -> BTreeMap<String, String> {
    let header = &info.header;
    let mut fields = BTreeMap::new();
    let mut put = |name: &str, value: Option<String>| {
        if let Some(value) = value.filter(|value| !value.is_empty()) {
            fields.insert(name.to_owned(), value);
        }
    };
    let join = |values: &[String]| Some(values.join(", "));
    put("name", header.name.clone());
    put("time_stamp", header.time_stamp.clone());
    put("author", join(&header.author));
    put("organization", join(&header.organization));
    put("preprocessor_version", header.preprocessor_version.clone());
    put("originating_system", header.originating_system.clone());
    put("authorization", header.authorization.clone());
    put("description", join(&header.description));
    put("implementation_level", header.implementation_level.clone());
    put("schemas", join(&header.schemas));
    // Units are a set: the same units declared in another order are equal.
    let sorted = |values: &[String]| {
        let mut values = values.to_vec();
        values.sort();
        join(&values)
    };
    put("units.length", sorted(&info.units.length));
    put("units.plane_angle", sorted(&info.units.plane_angle));
    put("units.solid_angle", sorted(&info.units.solid_angle));
    put("instances", Some(info.instances.to_string()));
    fields
}

/// Keyed values that differ, in key order.
fn fields(old: &BTreeMap<String, String>, new: &BTreeMap<String, String>) -> Vec<FieldChange> {
    let keys: BTreeSet<&String> = old.keys().chain(new.keys()).collect();
    keys.into_iter()
        .filter(|key| old.get(*key) != new.get(*key))
        .map(|key| FieldChange {
            field: key.clone(),
            old: old.get(key).cloned(),
            new: new.get(key).cloned(),
        })
        .collect()
}

fn entity_types(old: &Info, new: &Info) -> Vec<CountChange> {
    let count = |info: &Info| -> BTreeMap<String, usize> {
        info.entity_types
            .iter()
            .map(|entity| (entity.name.clone(), entity.count))
            .collect()
    };
    let (old, new) = (count(old), count(new));
    let names: BTreeSet<&String> = old.keys().chain(new.keys()).collect();
    names
        .into_iter()
        .filter_map(|name| {
            let (a, b) = (
                old.get(name).copied().unwrap_or(0),
                new.get(name).copied().unwrap_or(0),
            );
            (a != b).then(|| CountChange {
                entity_type: name.clone(),
                old: a,
                new: b,
            })
        })
        .collect()
}

/// A key per definition, in definition order: `product.id`, else the
/// product name, else `#id`, made unique with ` (2)`, ` (3)`, ….
fn definition_keys(structure: &ProductStructure) -> Vec<String> {
    let mut seen: BTreeMap<String, usize> = BTreeMap::new();
    structure
        .definitions()
        .iter()
        .map(|definition| {
            let product = definition.product.as_ref();
            let base = product
                .and_then(|p| p.id.clone().filter(|id| !id.is_empty()))
                .or_else(|| product.and_then(|p| p.name.clone().filter(|n| !n.is_empty())))
                .unwrap_or_else(|| format!("#{}", definition.instance));
            numbered(&mut seen, base)
        })
        .collect()
}

fn numbered(seen: &mut BTreeMap<String, usize>, base: String) -> String {
    let count = seen.entry(base.clone()).or_default();
    *count += 1;
    if *count == 1 {
        base
    } else {
        format!("{base} ({count})")
    }
}

fn definition_fields(definition: &Definition) -> BTreeMap<String, String> {
    let product = definition.product.as_ref();
    let mut fields = BTreeMap::new();
    let mut put = |name: &str, value: Option<&String>| {
        if let Some(value) = value.filter(|value| !value.is_empty()) {
            fields.insert(name.to_owned(), value.clone());
        }
    };
    put("name", product.and_then(|p| p.name.as_ref()));
    put("description", product.and_then(|p| p.description.as_ref()));
    put("version", definition.version.as_ref());
    put("definition.id", definition.id.as_ref());
    put("definition.description", definition.description.as_ref());
    fields
}

fn products(old: &ProductStructure, new: &ProductStructure) -> Vec<ProductChange> {
    let index = |structure: &ProductStructure| -> BTreeMap<String, BTreeMap<String, String>> {
        definition_keys(structure)
            .into_iter()
            .zip(structure.definitions().iter().map(definition_fields))
            .collect()
    };
    let (old, new) = (index(old), index(new));
    let keys: BTreeSet<&String> = old.keys().chain(new.keys()).collect();
    keys.into_iter()
        .filter_map(|key| {
            let change = |kind, fields| ProductChange {
                product: key.clone(),
                kind,
                fields,
            };
            match (old.get(key), new.get(key)) {
                (None, Some(_)) => Some(change(ChangeKind::Added, Vec::new())),
                (Some(_), None) => Some(change(ChangeKind::Removed, Vec::new())),
                (Some(a), Some(b)) if a != b => Some(change(ChangeKind::Changed, fields(a, b))),
                _ => None,
            }
        })
        .collect()
}

fn components(old: &ProductStructure, new: &ProductStructure) -> Vec<ComponentChange> {
    let quantities = |structure: &ProductStructure| -> BTreeMap<(String, String), f64> {
        let keys = definition_keys(structure);
        let mut quantities = BTreeMap::new();
        for usage in structure.usages() {
            let key = (keys[usage.parent].clone(), keys[usage.child].clone());
            *quantities.entry(key).or_insert(0.0) += usage.quantity.unwrap_or(1.0);
        }
        quantities
    };
    let (old, new) = (quantities(old), quantities(new));
    let keys: BTreeSet<&(String, String)> = old.keys().chain(new.keys()).collect();
    keys.into_iter()
        .filter_map(|key| {
            let (a, b) = (
                old.get(key).copied().unwrap_or(0.0),
                new.get(key).copied().unwrap_or(0.0),
            );
            ((a - b).abs() > f64::EPSILON).then(|| ComponentChange {
                parent: key.0.clone(),
                child: key.1.clone(),
                old: a,
                new: b,
            })
        })
        .collect()
}

/// `(product key, property key)` → value, for every property value.
fn property_values(
    graph: &Graph<'_>,
    structure: &ProductStructure,
) -> BTreeMap<(String, String), String> {
    let keys = definition_keys(structure);
    let key_of = |definition: Option<u64>| {
        definition
            .and_then(|id| {
                structure
                    .definitions()
                    .iter()
                    .position(|d| d.instance == id)
            })
            .map(|index| keys[index].clone())
            .unwrap_or_default()
    };
    let mut seen: BTreeMap<String, usize> = BTreeMap::new();
    let mut values = BTreeMap::new();
    for property in properties(graph, structure).properties {
        let product = key_of(property.subject.product_definition);
        let label = without_instance_names(property.label());
        let entries: Vec<(String, String)> = if property.values.is_empty() {
            vec![(label, String::new())]
        } else {
            property
                .values
                .iter()
                .map(|value| {
                    let name = value.name.as_deref().unwrap_or_default();
                    let key = if name.is_empty() || name == property.label() {
                        label.clone()
                    } else {
                        format!("{label} / {}", without_instance_names(name))
                    };
                    (key, without_instance_names(&value.value))
                })
                .collect()
        };
        for (key, value) in entries {
            let key = numbered(&mut seen, format!("{product}\u{0}{key}"));
            let key = key.split_once('\u{0}').map_or_else(
                || (product.clone(), key.clone()),
                |(product, key)| (product.to_owned(), key.to_owned()),
            );
            values.insert(key, value);
        }
    }
    values
}

fn property_changes(
    old: &BTreeMap<(String, String), String>,
    new: &BTreeMap<(String, String), String>,
) -> Vec<PropertyChange> {
    let keys: BTreeSet<&(String, String)> = old.keys().chain(new.keys()).collect();
    keys.into_iter()
        .filter_map(|key| {
            let (a, b) = (old.get(key), new.get(key));
            let kind = match (a, b) {
                (None, Some(_)) => ChangeKind::Added,
                (Some(_), None) => ChangeKind::Removed,
                (Some(a), Some(b)) if a != b => ChangeKind::Changed,
                _ => return None,
            };
            Some(PropertyChange {
                product: key.0.clone(),
                property: key.1.clone(),
                kind,
                old: a.cloned(),
                new: b.cloned(),
            })
        })
        .collect()
}

/// `text` with every `#123` replaced by `#`, so that the same property in
/// two exports compares equal.
fn without_instance_names(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        out.push(c);
        if c == '#' {
            while chars.peek().is_some_and(char::is_ascii_digit) {
                chars.next();
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::p21::parse;

    const OLD: &str = "ISO-10303-21;HEADER;
FILE_NAME('a.stp','2026-01-01',(''),(''),'','CAD 1','');
FILE_SCHEMA(('AP242'));ENDSEC;DATA;
#1=APPLICATION_CONTEXT('');
#2=PRODUCT_DEFINITION_CONTEXT('',#1,'design');
#10=PRODUCT('A','assembly','',());
#11=PRODUCT_DEFINITION_FORMATION('1','',#10);
#12=PRODUCT_DEFINITION('design','',#11,#2);
#20=PRODUCT('B','bolt','',());
#21=PRODUCT_DEFINITION_FORMATION('1','',#20);
#22=PRODUCT_DEFINITION('design','',#21,#2);
#30=PRODUCT('C','nut','',());
#31=PRODUCT_DEFINITION_FORMATION('1','',#30);
#32=PRODUCT_DEFINITION('design','',#31,#2);
#40=NEXT_ASSEMBLY_USAGE_OCCURRENCE('1','','',#12,#22,$);
#41=NEXT_ASSEMBLY_USAGE_OCCURRENCE('2','','',#12,#22,$);
#42=NEXT_ASSEMBLY_USAGE_OCCURRENCE('3','','',#12,#32,$);
#50=PROPERTY_DEFINITION('geometric_validation_property','volume of #99',#22);
#51=PROPERTY_DEFINITION_REPRESENTATION(#50,#52);
#52=REPRESENTATION('',(#53),#1);
#53=MEASURE_REPRESENTATION_ITEM('volume measure',VOLUME_MEASURE(10.),#1);
ENDSEC;END-ISO-10303-21;";

    fn graphs(new: &str) -> Diff {
        let old = Graph::new(parse(OLD.as_bytes()).unwrap()).unwrap();
        let new = Graph::new(parse(new.as_bytes()).unwrap()).unwrap();
        diff(&old, &new)
    }

    #[test]
    fn a_file_has_no_differences_with_itself() {
        let diff = graphs(OLD);
        assert!(diff.is_empty(), "{diff:?}");
    }

    #[test]
    fn renumbering_is_not_a_difference() {
        // Every instance name shifted by 1000, including the one inside the
        // validation property's description.
        let mut renumbered = String::new();
        let mut rest = OLD;
        while let Some(at) = rest.find('#') {
            renumbered.push_str(&rest[..=at]);
            rest = &rest[at + 1..];
            let digits = rest.chars().take_while(char::is_ascii_digit).count();
            let id: u64 = rest[..digits].parse().unwrap();
            renumbered.push_str(&(id + 1000).to_string());
            rest = &rest[digits..];
        }
        renumbered.push_str(rest);
        let diff = graphs(&renumbered);
        assert!(diff.is_empty(), "{diff:?}");
    }

    #[test]
    fn every_section_reports_its_changes() {
        let new = OLD
            .replace("'CAD 1'", "'CAD 2'")
            .replace("PRODUCT('B','bolt'", "PRODUCT('B','hex bolt'")
            .replace(
                "#42=NEXT_ASSEMBLY_USAGE_OCCURRENCE('3','','',#12,#32,$);\n",
                "",
            )
            .replace("VOLUME_MEASURE(10.)", "VOLUME_MEASURE(12.5)")
            .replace(
                "#30=PRODUCT('C','nut','',());",
                "#30=PRODUCT('D','washer','',());",
            );
        let diff = graphs(&new);
        assert_eq!(
            diff.header,
            [
                FieldChange {
                    field: "instances".to_owned(),
                    old: Some("18".to_owned()),
                    new: Some("17".to_owned()),
                },
                FieldChange {
                    field: "originating_system".to_owned(),
                    old: Some("CAD 1".to_owned()),
                    new: Some("CAD 2".to_owned()),
                },
            ]
        );
        assert_eq!(
            diff.entity_types,
            [CountChange {
                entity_type: "NEXT_ASSEMBLY_USAGE_OCCURRENCE".to_owned(),
                old: 3,
                new: 2,
            }]
        );
        let products: Vec<(&str, ChangeKind)> = diff
            .products
            .iter()
            .map(|change| (change.product.as_str(), change.kind))
            .collect();
        assert_eq!(
            products,
            [
                ("B", ChangeKind::Changed),
                ("C", ChangeKind::Removed),
                ("D", ChangeKind::Added),
            ]
        );
        assert_eq!(
            diff.products[0].fields,
            [FieldChange {
                field: "name".to_owned(),
                old: Some("bolt".to_owned()),
                new: Some("hex bolt".to_owned()),
            }]
        );
        assert_eq!(
            diff.components,
            [ComponentChange {
                parent: "A".to_owned(),
                child: "C".to_owned(),
                old: 1.0,
                new: 0.0,
            }]
        );
        assert_eq!(
            diff.properties,
            [PropertyChange {
                product: "B".to_owned(),
                property: "volume of # / volume measure".to_owned(),
                kind: ChangeKind::Changed,
                old: Some("10.".to_owned()),
                new: Some("12.5".to_owned()),
            }]
        );
        assert_eq!(diff.len(), 8);
    }
}
