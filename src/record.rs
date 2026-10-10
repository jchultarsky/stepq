//! Shared helpers for reading records: entity names, partial records,
//! attribute counts, numbers, representations and the containers that hold
//! topology.
//!
//! Every reader of stated values goes through these, so that "which record
//! of a complex instance holds this attribute", "how many attributes must it
//! have" and "which shell, solid or model holds what" are answered once.
//! `lint` keeps its own looser reader on purpose: it reports malformed
//! files, so it must not refuse what it is reporting on.

use std::borrow::Cow;

use crate::p21::{Exchange, Instance, Param, Record};

/// The upper-cased entity types of an instance; for a complex instance, its
/// partial entities joined with `+`, in the order written.
pub(crate) fn entity(exchange: &Exchange<'_>, instance: &Instance) -> String {
    exchange
        .records(instance)
        .map(|record| upper(record.name()))
        .collect::<Vec<_>>()
        .join("+")
}

/// An entity type name, upper-cased.
pub(crate) fn upper(name: &[u8]) -> String {
    String::from_utf8_lossy(name).to_ascii_uppercase()
}

/// The record named `name`, ignoring ASCII case: the record of a simple
/// instance of that type, or that partial entity of a complex instance.
pub(crate) fn partial<'e>(
    exchange: &'e Exchange<'_>,
    instance: &Instance,
    name: &str,
) -> Option<Record<'e>> {
    exchange.records(instance).find(|r| r.is(name))
}

/// True if a complex instance writes some partial entity more than once.
///
/// A complex instance is a set of entity types (ISO 10303-11), so ISO
/// 10303-21 writes each partial once. A repeated partial states its
/// attributes twice, and a reader that took either one would be guessing,
/// so every reader of stated values refuses such an instance.
pub(crate) fn repeats_partial(exchange: &Exchange<'_>, instance: &Instance) -> bool {
    if !instance.is_complex() {
        return false;
    }
    let mut names: Vec<Vec<u8>> = exchange
        .records(instance)
        .map(|record| record.name().to_ascii_uppercase())
        .collect();
    names.sort_unstable();
    names.windows(2).any(|pair| pair[0] == pair[1])
}

/// The partial record named `name`, or failing that the first record.
pub(crate) fn primary_record<'e>(
    exchange: &'e Exchange<'_>,
    instance: &Instance,
    name: &str,
) -> Option<Record<'e>> {
    let mut records = exchange.records(instance);
    records
        .clone()
        .find(|r| r.is(name))
        .or_else(|| records.next())
}

/// The record's parameters if it has exactly `count`, `None` otherwise.
///
/// Readers of stated values check the attribute count of every record or
/// partial record they read: a record with another count does not have the
/// layout the reader assumes.
pub(crate) fn exact_params<'e>(record: Record<'e>, count: usize) -> Option<Vec<Param<'e>>> {
    let params: Vec<Param<'e>> = record.params().collect();
    (params.len() == count).then_some(params)
}

/// The referenced `#id`, if the parameter is a reference.
pub(crate) fn reference(param: Option<&Param<'_>>) -> Option<u64> {
    param.and_then(Param::reference)
}

/// An INTEGER or REAL token as a finite number. `None` for anything else,
/// including a typed value and a number too large for an `f64`.
pub(crate) fn real(param: &Param<'_>) -> Option<f64> {
    match param {
        Param::Integer(literal) | Param::Real(literal) => {
            literal.to_f64().filter(|value| value.is_finite())
        }
        _ => None,
    }
}

/// A string attribute, decoded; `None` if unset or not a string.
pub(crate) fn text(param: Option<Param<'_>>) -> Option<String> {
    let literal = param?.literal()?;
    if literal.text().first() != Some(&b'\'') {
        return None;
    }
    literal.decode().ok().map(Cow::into_owned)
}

/// What `representation` reads: `representation.items` and
/// `representation.context_of_items`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Representation {
    /// The `#id`s of the items, in order.
    pub(crate) items: Vec<u64>,
    /// The `#id` of the context, which is not checked here.
    pub(crate) context: u64,
}

/// Why [`representation`] could not read an instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NotRepresentation {
    /// No instance has this `#id`.
    Undefined,
    /// The instance does not have the layout of a representation.
    Layout,
}

/// Reads instance `#id` as a representation: `(name, items, context)`.
///
/// A simple instance is read from its first three attributes, which every
/// subtype of `representation` inherits in that order; a subtype may add
/// attributes after them (`tessellated_shape_representation_with_accuracy_parameters`
/// adds a fourth). A complex instance is read from its `REPRESENTATION`
/// partial, which must have exactly three, and one that repeats any partial
/// is refused ([`repeats_partial`]). Either way `name` must be a string,
/// `items` a list of references and `context_of_items` a reference.
pub(crate) fn representation(
    exchange: &Exchange<'_>,
    id: u64,
) -> Result<Representation, NotRepresentation> {
    let instance = exchange.get(id).ok_or(NotRepresentation::Undefined)?;
    let params: Vec<Param<'_>> = if instance.is_complex() {
        if repeats_partial(exchange, instance) {
            return Err(NotRepresentation::Layout);
        }
        partial(exchange, instance, "REPRESENTATION")
            .and_then(|record| exact_params(record, 3))
            .ok_or(NotRepresentation::Layout)?
    } else {
        let record = exchange
            .records(instance)
            .next()
            .ok_or(NotRepresentation::Layout)?;
        let params: Vec<Param<'_>> = record.params().take(3).collect();
        if params.len() < 3 {
            return Err(NotRepresentation::Layout);
        }
        params
    };
    let [name, items, context] = params.as_slice() else {
        return Err(NotRepresentation::Layout);
    };
    if !matches!(name, Param::String(_)) {
        return Err(NotRepresentation::Layout);
    }
    let items = items
        .list()
        .ok_or(NotRepresentation::Layout)?
        .map(|item| item.reference())
        .collect::<Option<Vec<u64>>>()
        .ok_or(NotRepresentation::Layout)?;
    let context = context.reference().ok_or(NotRepresentation::Layout)?;
    Ok(Representation { items, context })
}

/// True if the record is a representation context or one of its subtypes,
/// such as `GEOMETRIC_REPRESENTATION_CONTEXT` or
/// `GLOBAL_UNIT_ASSIGNED_CONTEXT`.
pub(crate) fn is_context_record(record: Record<'_>) -> bool {
    let name = upper(record.name());
    name.ends_with("REPRESENTATION_CONTEXT") || name.ends_with("ASSIGNED_CONTEXT")
}

/// `shape_aspect` and its subtypes in the four schemas whose simple
/// records keep `of_shape` as attribute 2, as `shape_aspect` itself does.
/// The few that list another supertype first (the assembly joints, an
/// AP242 `instanced_feature`) are not here.
pub(crate) const SHAPE_ASPECTS: &[&str] = &[
    "ALL_AROUND_SHAPE_ASPECT",
    "APEX",
    "APPLIED_AREA",
    "ASSEMBLY_BOND_DEFINITION",
    "ASSEMBLY_SHAPE_CONSTRAINT",
    "ASSEMBLY_SHAPE_JOINT",
    "BASIC_ROUND_HOLE_OCCURRENCE",
    "BASIC_ROUND_HOLE_OCCURRENCE_IN_ASSEMBLY",
    "BEAD_END",
    "BETWEEN_SHAPE_ASPECT",
    "BOSS_TOP",
    "CENTRE_OF_SYMMETRY",
    "CHAMFER",
    "CHAMFER_OFFSET",
    "CIRCULAR_CLOSED_PROFILE",
    "CLOSED_PATH_PROFILE",
    "COMMON_DATUM",
    "COMPONENT_FEATURE",
    "COMPONENT_MATING_CONSTRAINT_CONDITION",
    "COMPONENT_PATH_SHAPE_ASPECT",
    "COMPONENT_TERMINAL",
    "COMPOSITE_GROUP_SHAPE_ASPECT",
    "COMPOSITE_SHAPE_ASPECT",
    "COMPOSITE_UNIT_SHAPE_ASPECT",
    "CONNECTION_ZONE_INTERFACE_PLANE_RELATIONSHIP",
    "CONNECTIVITY_DEFINITION",
    "CONSTITUENT_SHAPE_ASPECT",
    "CONTACTING_FEATURE",
    "CONTACT_FEATURE",
    "CONTINUOUS_SHAPE_ASPECT",
    "COUNTERBORE_HOLE_OCCURRENCE",
    "COUNTERBORE_HOLE_OCCURRENCE_IN_ASSEMBLY",
    "COUNTERDRILL_HOLE_OCCURRENCE",
    "COUNTERDRILL_HOLE_OCCURRENCE_IN_ASSEMBLY",
    "COUNTERSINK_HOLE_OCCURRENCE",
    "COUNTERSINK_HOLE_OCCURRENCE_IN_ASSEMBLY",
    "CROSS_SECTIONAL_ALTERNATIVE_SHAPE_ELEMENT",
    "CROSS_SECTIONAL_GROUP_SHAPE_ELEMENT",
    "CROSS_SECTIONAL_GROUP_SHAPE_ELEMENT_WITH_LACING",
    "CROSS_SECTIONAL_GROUP_SHAPE_ELEMENT_WITH_TUBULAR_COVER",
    "CROSS_SECTIONAL_OCCURRENCE_SHAPE_ELEMENT",
    "CROSS_SECTIONAL_PART_SHAPE_ELEMENT",
    "DATUM",
    "DATUM_FEATURE",
    "DATUM_REFERENCE_COMPARTMENT",
    "DATUM_REFERENCE_ELEMENT",
    "DATUM_SYSTEM",
    "DATUM_SYSTEM_FOR_COMPOSITE_GROUP_ELEMENT",
    "DATUM_TARGET",
    "DERIVED_SHAPE_ASPECT",
    "DIMENSIONAL_LOCATION_WITH_DATUM_FEATURE",
    "DIMENSIONAL_SIZE_WITH_DATUM_FEATURE",
    "DIRECTED_ANGLE",
    "DIRECTED_TOLERANCE_ZONE",
    "DIRECTION_FEATURE_TOLERANCE_ZONE",
    "EDGE_ROUND",
    "EXTENSION",
    "FACE_SURFACE_SHAPE_ASPECT",
    "FILLET",
    "GENERAL_DATUM_REFERENCE",
    "GEOMETRIC_ALIGNMENT",
    "GEOMETRIC_CONTACT",
    "GEOMETRIC_INTERSECTION",
    "HARNESS_NODE",
    "HARNESS_SEGMENT",
    "HOLE_BOTTOM",
    "JOGGLE_TERMINATION",
    "LINEAR_PROFILE",
    "MODIFIED_PATTERN",
    "NGON_CLOSED_PROFILE",
    "OPEN_PATH_PROFILE",
    "ORIENTED_TOLERANCE_ZONE",
    "PARALLEL_OFFSET",
    "PARTIAL_CIRCULAR_PROFILE",
    "PATH_FEATURE_COMPONENT",
    "PERPENDICULAR_TO",
    "PHYSICAL_COMPONENT_FEATURE",
    "PHYSICAL_COMPONENT_INTERFACE_TERMINAL",
    "PHYSICAL_COMPONENT_TERMINAL",
    "PLACED_DATUM_TARGET_FEATURE",
    "PLACED_FEATURE",
    "POCKET_BOTTOM",
    "PROFILE_FLOOR",
    "RECTANGULAR_CLOSED_PROFILE",
    "RIB_TOP_FLOOR",
    "ROUNDED_U_PROFILE",
    "SHAPE_ASPECT",
    "SHAPE_ASPECT_OCCURRENCE",
    "SLOT_END",
    "SPOTFACE_OCCURRENCE",
    "SPOTFACE_OCCURRENCE_IN_ASSEMBLY",
    "SQUARE_U_PROFILE",
    "SURFACE_SHAPE_ASPECT",
    "SYMMETRIC_SHAPE_ASPECT",
    "TANGENT",
    "TAPER",
    "TEE_PROFILE",
    "TERMINAL_FEATURE",
    "TERMINAL_LOCATION_GROUP",
    "THREAD_RUNOUT",
    "TOLERANCE_ZONE",
    "TOLERANCE_ZONE_WITH_DATUM",
    "TRANSITION_FEATURE",
    "TRANSPORT_FEATURE",
    "TWISTED_CROSS_SECTIONAL_GROUP_SHAPE_ELEMENT",
    "VEE_PROFILE",
];

/// A topological container: an entity that holds faces, shells or other
/// containers in some of its attributes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Container {
    /// The entity type, upper case.
    pub(crate) entity: &'static str,
    /// Its exact attribute count, as a simple instance.
    pub(crate) params: usize,
    /// The attributes that hold its parts, by position: a reference or a
    /// list of references.
    pub(crate) holds: &'static [usize],
}

/// Which attribute of which face, shell, solid or model holds what (ISO
/// 10303-42). Units and topology both walk containment through this table,
/// so the two cannot disagree about what holds a face. A walk may still
/// refuse to read what a container holds (topology, for one, does not read
/// an `oriented_face`'s bounds); the table only says what is held.
///
/// Only simple instances are containers. A complex instance with a partial
/// of one of these types is not followed, and a walk that meets one reports
/// it instead of guessing what it holds.
pub(crate) const CONTAINERS: &[Container] = &[
    // connected_face_set.cfs_faces
    Container {
        entity: "CLOSED_SHELL",
        params: 2,
        holds: &[1],
    },
    Container {
        entity: "OPEN_SHELL",
        params: 2,
        holds: &[1],
    },
    Container {
        entity: "CONNECTED_FACE_SET",
        params: 2,
        holds: &[1],
    },
    // connected_face_sub_set(name, cfs_faces, parent_face_set): the parent
    // holds the subset, not the other way round.
    Container {
        entity: "CONNECTED_FACE_SUB_SET",
        params: 3,
        holds: &[1],
    },
    // oriented_face(name, *, face_element, orientation): bounds is derived
    // and written `*`.
    Container {
        entity: "ORIENTED_FACE",
        params: 4,
        holds: &[2],
    },
    // oriented_closed_shell(name, *, closed_shell_element, orientation):
    // cfs_faces is derived and written `*`.
    Container {
        entity: "ORIENTED_CLOSED_SHELL",
        params: 4,
        holds: &[2],
    },
    Container {
        entity: "ORIENTED_OPEN_SHELL",
        params: 4,
        holds: &[2],
    },
    // manifold_solid_brep.outer
    Container {
        entity: "MANIFOLD_SOLID_BREP",
        params: 2,
        holds: &[1],
    },
    Container {
        entity: "FACETED_BREP",
        params: 2,
        holds: &[1],
    },
    // brep_with_voids(name, outer, voids)
    Container {
        entity: "BREP_WITH_VOIDS",
        params: 3,
        holds: &[1, 2],
    },
    // shell_based_surface_model.sbsm_boundary
    Container {
        entity: "SHELL_BASED_SURFACE_MODEL",
        params: 2,
        holds: &[1],
    },
    // face_based_surface_model.fbsm_faces
    Container {
        entity: "FACE_BASED_SURFACE_MODEL",
        params: 2,
        holds: &[1],
    },
];

/// The container entry for a record, if it is a simple instance of a
/// container type.
pub(crate) fn container(record: Record<'_>) -> Option<&'static Container> {
    CONTAINERS
        .iter()
        .find(|container| record.is(container.entity))
}

/// The `#id`s a container attribute holds: the reference itself, or every
/// reference in a list.
pub(crate) fn held(param: &Param<'_>) -> Vec<u64> {
    match param {
        Param::Reference(target) => vec![*target],
        Param::List(items) => items.clone().filter_map(|item| item.reference()).collect(),
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::p21::parse;

    fn exchange(data: &str) -> Exchange<'static> {
        let src = format!(
            "ISO-10303-21;HEADER;FILE_SCHEMA(('X'));ENDSEC;DATA;{data}ENDSEC;END-ISO-10303-21;"
        );
        let src: &'static [u8] = Box::leak(src.into_bytes().into_boxed_slice());
        parse(src).unwrap()
    }

    #[test]
    fn entities_and_partials() {
        let x = exchange(
            "#1=(LENGTH_UNIT()NAMED_UNIT(*)SI_UNIT(.MILLI.,.METRE.));#2=product('a','b','',());",
        );
        let unit = x.get(1).unwrap();
        assert_eq!(entity(&x, unit), "LENGTH_UNIT+NAMED_UNIT+SI_UNIT");
        assert_eq!(entity(&x, x.get(2).unwrap()), "PRODUCT");
        let si = partial(&x, unit, "si_unit").unwrap();
        assert_eq!(si.name(), b"SI_UNIT");
        assert!(partial(&x, unit, "MASS_UNIT").is_none());
        assert_eq!(
            primary_record(&x, unit, "MASS_UNIT").unwrap().name(),
            b"LENGTH_UNIT",
            "falls back to the first record"
        );
        assert_eq!(exact_params(si, 2).map(|p| p.len()), Some(2));
        assert!(exact_params(si, 3).is_none());
    }

    #[test]
    fn numbers_and_strings() {
        let x = exchange("#1=A(1,2.5,1.E999,'x',$,LENGTH_MEASURE(3.),'it''s');");
        let record = x.records(x.get(1).unwrap()).next().unwrap();
        let params: Vec<Param<'_>> = record.params().collect();
        assert_eq!(real(&params[0]), Some(1.0), "an INTEGER is a number");
        assert_eq!(real(&params[1]), Some(2.5));
        assert_eq!(real(&params[2]), None, "not finite");
        assert_eq!(real(&params[3]), None);
        assert_eq!(real(&params[4]), None);
        assert_eq!(real(&params[5]), None, "a typed value is not a bare number");
        assert_eq!(text(record.param(6)).as_deref(), Some("it's"));
        assert_eq!(text(record.param(4)), None);
        assert_eq!(reference(record.param(0).as_ref()), None);
    }

    #[test]
    fn representations_simple_complex_and_subtypes() {
        let x = exchange(
            "#1=SHAPE_REPRESENTATION('',(#5,#6),#9);\
             #2=(REPRESENTATION('',(#5),#9)SHAPE_REPRESENTATION());\
             #3=TESSELLATED_SHAPE_REPRESENTATION_WITH_ACCURACY_PARAMETERS('',(#5),#9,(0.1));\
             #4=SHAPE_DEFINITION_REPRESENTATION(#7,#1);\
             #8=(REPRESENTATION('',(#5),#9,#9)SHAPE_REPRESENTATION());\
             #10=SHAPE_REPRESENTATION('',(#5,'x'),#9);\
             #11=SHAPE_REPRESENTATION($,(#5),#9);",
        );
        assert_eq!(
            representation(&x, 1),
            Ok(Representation {
                items: vec![5, 6],
                context: 9
            })
        );
        assert_eq!(representation(&x, 2).map(|r| r.items), Ok(vec![5]));
        assert_eq!(
            representation(&x, 3).map(|r| r.context),
            Ok(9),
            "a subtype's own attributes come after the inherited three"
        );
        assert_eq!(representation(&x, 4), Err(NotRepresentation::Layout));
        assert_eq!(
            representation(&x, 8),
            Err(NotRepresentation::Layout),
            "a REPRESENTATION partial has exactly three attributes"
        );
        assert_eq!(representation(&x, 10), Err(NotRepresentation::Layout));
        assert_eq!(representation(&x, 11), Err(NotRepresentation::Layout));
        assert_eq!(representation(&x, 99), Err(NotRepresentation::Undefined));
    }

    #[test]
    fn containers() {
        let x = exchange(
            "#1=CLOSED_SHELL('',(#5,#6));#2=ORIENTED_CLOSED_SHELL('',*,#1,.F.);\
             #3=BREP_WITH_VOIDS('',#1,(#2));#4=ADVANCED_FACE('',(),#9,.T.);\
             #5=ORIENTED_FACE('',*,#4,.F.);#6=CONNECTED_FACE_SUB_SET('',(#4),#1);",
        );
        let record = |id| x.records(x.get(id).unwrap()).next().unwrap();
        let shell = container(record(1)).unwrap();
        assert_eq!(held(&record(1).param(shell.holds[0]).unwrap()), [5, 6]);
        assert_eq!(container(record(2)).unwrap().holds, &[2]);
        let voids = container(record(3)).unwrap();
        let held_by_voids: Vec<u64> = voids
            .holds
            .iter()
            .flat_map(|&i| held(&record(3).param(i).unwrap()))
            .collect();
        assert_eq!(held_by_voids, [1, 2]);
        assert!(container(record(4)).is_none());
        assert_eq!(container(record(5)).unwrap().holds, &[2], "the face");
        assert_eq!(
            container(record(6)).unwrap().holds,
            &[1],
            "the faces, not the parent"
        );
        assert_eq!(held(&record(4).param(0).unwrap()), Vec::<u64>::new());
    }

    #[test]
    fn repeated_partials() {
        let x = exchange(
            "#1=(LENGTH_UNIT()NAMED_UNIT(*)SI_UNIT(.MILLI.,.METRE.));\
             #2=(LENGTH_UNIT()NAMED_UNIT(*)SI_UNIT(.MILLI.,.METRE.)si_unit(.CENTI.,.METRE.));\
             #3=SI_UNIT(*,.MILLI.,.METRE.);\
             #4=(REPRESENTATION('',(#5),#9)REPRESENTATION('',(#5),#8)SHAPE_REPRESENTATION());\
             #6=(REPRESENTATION('',(#5),#9)SHAPE_REPRESENTATION()SHAPE_REPRESENTATION());",
        );
        assert!(!repeats_partial(&x, x.get(1).unwrap()));
        assert!(
            repeats_partial(&x, x.get(2).unwrap()),
            "names are compared ignoring case"
        );
        assert!(!repeats_partial(&x, x.get(3).unwrap()));
        assert_eq!(
            representation(&x, 4),
            Err(NotRepresentation::Layout),
            "two contexts: neither is read"
        );
        assert_eq!(representation(&x, 6), Err(NotRepresentation::Layout));
    }

    /// `SHAPE_ASPECTS` is exactly the subtypes of `shape_aspect` whose
    /// `of_shape` is attribute 2 in every schema (`tools/fetch-schemas.sh`;
    /// skipped without them, failed with `STEPQ_REQUIRE_FIXTURES=1`).
    #[test]
    fn shape_aspects_keep_of_shape_third() {
        use crate::express::Schema;
        use std::collections::{BTreeMap, BTreeSet};
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/schemas");
        let mut layouts: BTreeMap<String, bool> = BTreeMap::new();
        let mut read = 0;
        for file in [
            "ap242e4_mim_lf.exp",
            "ap214e3.exp",
            "ap203e2_mim_lf.exp",
            "ap203.exp",
        ] {
            let Ok(src) = std::fs::read(dir.join(file)) else {
                continue;
            };
            read += 1;
            let schema = Schema::parse(&src).unwrap();
            let mut found = BTreeSet::from(["SHAPE_ASPECT".to_owned()]);
            let mut stack = vec!["SHAPE_ASPECT".to_owned()];
            while let Some(name) = stack.pop() {
                for entity in schema.entities() {
                    if entity
                        .supertypes
                        .iter()
                        .any(|s| s.eq_ignore_ascii_case(&name))
                        && found.insert(entity.name.clone())
                    {
                        stack.push(entity.name.clone());
                    }
                }
            }
            for name in found {
                let third = schema
                    .slots(&name)
                    .unwrap()
                    .get(2)
                    .map(|slot| slot.name.clone());
                let ok = third.as_deref() == Some("OF_SHAPE");
                *layouts.entry(name).or_insert(true) &= ok;
            }
        }
        let required =
            std::env::var_os("STEPQ_REQUIRE_FIXTURES").is_some_and(|v| v != "0" && !v.is_empty());
        if read < 4 {
            assert!(
                !required,
                "{read} of 4 schemas (run tools/fetch-schemas.sh)"
            );
            return;
        }
        let expected: BTreeSet<&str> = layouts
            .iter()
            .filter(|(_, ok)| **ok)
            .map(|(name, _)| name.as_str())
            .collect();
        let listed: BTreeSet<&str> = SHAPE_ASPECTS.iter().copied().collect();
        assert_eq!(listed, expected);
    }

    #[test]
    fn context_records() {
        let x = exchange(
            "#1=GLOBAL_UNIT_ASSIGNED_CONTEXT('','',(#9));\
             #2=(GEOMETRIC_REPRESENTATION_CONTEXT(3)REPRESENTATION_CONTEXT('',''));\
             #3=SHAPE_REPRESENTATION('',(),#2);",
        );
        let any_context = |id| x.records(x.get(id).unwrap()).any(is_context_record);
        assert!(any_context(1));
        assert!(any_context(2));
        assert!(!any_context(3));
    }
}
