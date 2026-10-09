//! Unit factors as the file states them. A unit nobody states is an error,
//! never a millimetre.
//!
//! stepq reads lengths in millimetres, plane angles in radians and solid
//! angles in steradians, each converted with the factor of the unit the
//! file states for the value's own representation context. This module
//! reads those factors:
//!
//! * [`unit()`] reads one named unit by `#id`. An SI unit takes its factor
//!   from an exact table of the 16 SI prefixes; a conversion-based unit's
//!   factor is its stated value times its base unit's factor, however deep
//!   the chain (up to 8). Nothing is snapped: a degree defined as
//!   `0.0174532925` radian is that, not π/180, and a millimetre is exactly
//!   1.
//! * [`ContextUnits`] reads what one representation context assigns: its
//!   unit of length, of plane angle and of solid angle, each read or refused
//!   on its own, so that a broken angle unit does not stop anyone reading
//!   lengths; its stated uncertainties; and its coordinate space dimension.
//!   [`ContextUnits::of_representation`] reads the context of a
//!   representation, and [`ContextUnits::of_item`] the contexts of every
//!   representation that holds a solid, shell or face, which must agree.
//!
//! What cannot be read as stated is a [`UnitError`] that names the
//! innermost instance and the reason. Nothing is defaulted: a context that
//! states no length unit has none.
//!
//! [`info::Units`](crate::info::Units) lists the unit names a file declares
//! anywhere, for display; the names here are labelled the same way.
//!
//! ```
//! use stepq::model::Graph;
//! use stepq::p21::parse;
//! use stepq::units::ContextUnits;
//!
//! let src = b"ISO-10303-21;HEADER;FILE_SCHEMA(('AP214'));ENDSEC;DATA;
//! #1=(LENGTH_UNIT()NAMED_UNIT(*)SI_UNIT(.MILLI.,.METRE.));
//! #2=LENGTH_MEASURE_WITH_UNIT(LENGTH_MEASURE(25.4),#1);
//! #3=DIMENSIONAL_EXPONENTS(1.,0.,0.,0.,0.,0.,0.);
//! #4=(CONVERSION_BASED_UNIT('INCH',#2)LENGTH_UNIT()NAMED_UNIT(#3));
//! #5=(NAMED_UNIT(*)PLANE_ANGLE_UNIT()SI_UNIT($,.RADIAN.));
//! #6=UNCERTAINTY_MEASURE_WITH_UNIT(LENGTH_MEASURE(1.E-3),#4,'closure','');
//! #7=(GEOMETRIC_REPRESENTATION_CONTEXT(3)GLOBAL_UNCERTAINTY_ASSIGNED_CONTEXT((#6))
//!   GLOBAL_UNIT_ASSIGNED_CONTEXT((#4,#5))REPRESENTATION_CONTEXT('',''));
//! #8=SHAPE_REPRESENTATION('',(),#7);
//! ENDSEC;END-ISO-10303-21;";
//!
//! let graph = Graph::new(parse(src)?)?;
//! let units = ContextUnits::of_representation(&graph, 8)?;
//! let inch = units.length()?;
//! assert_eq!((inch.name.as_str(), inch.factor), ("inch", 25.4));
//! assert_eq!(units.plane_angle()?.factor, 1.0);
//! assert!(units.solid_angle().is_err(), "no solid angle unit is stated");
//! assert_eq!(units.uncertainty()?[0].value, 1.0e-3 * 25.4);
//! assert_eq!(units.dimension, Some(3));
//! # Ok::<(), stepq::Error>(())
//! ```

use std::collections::HashSet;
use std::fmt;

use crate::model::Graph;
use crate::p21::{Exchange, Instance, Literal, Param, Record};
use crate::policy::MAX_CONVERSION_DEPTH;
use crate::record;

/// What a unit measures. Each kind has one canonical unit, which every
/// factor converts to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
#[non_exhaustive]
pub enum UnitKind {
    /// A length; canonical unit the millimetre.
    Length,
    /// A plane angle; canonical unit the radian.
    PlaneAngle,
    /// A solid angle; canonical unit the steradian.
    SolidAngle,
}

impl UnitKind {
    const ALL: [Self; 3] = [Self::Length, Self::PlaneAngle, Self::SolidAngle];

    fn slot(self) -> usize {
        match self {
            Self::Length => 0,
            Self::PlaneAngle => 1,
            Self::SolidAngle => 2,
        }
    }
}

impl fmt::Display for UnitKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Length => "length",
            Self::PlaneAngle => "plane angle",
            Self::SolidAngle => "solid angle",
        })
    }
}

/// A named unit of length, plane angle or solid angle and its factor.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct Unit {
    /// The `#id` of the named unit, simple or complex.
    pub instance: u64,
    /// What it measures.
    pub kind: UnitKind,
    /// Lower case, labelled as [`info::Units`](crate::info::Units) labels
    /// it: an SI unit with its prefix (`millimetre`, `radian`), or a
    /// conversion-based unit's name as written (`inch`, `degree`,
    /// `degrees`). The name is reported, never interpreted.
    pub name: String,
    /// A value in this unit times `factor` is the value in millimetres,
    /// radians or steradians: an SI prefix's exact factor, or a
    /// conversion-based unit's stated value times its base unit's factor.
    /// Always finite and positive.
    pub factor: f64,
}

/// An `uncertainty_measure_with_unit` that a context states.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct Uncertainty {
    /// The `#id` of the `uncertainty_measure_with_unit`.
    pub instance: u64,
    /// Its name as written, such as `distance_accuracy_value` or `closure`.
    /// Exporters differ.
    pub name: String,
    /// Its description as written; `None` if unset.
    pub description: Option<String>,
    /// The value in millimetres (or radians, or steradians), converted with
    /// its own unit, which need not be the context's. Always finite and
    /// positive.
    pub value: f64,
    /// Its own unit.
    pub unit: Unit,
}

/// The units and uncertainties that one representation context assigns.
///
/// Each kind of unit and the uncertainties are read, or refused with a
/// reason, on their own: a context whose solid angle unit is missing still
/// has a length unit.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct ContextUnits {
    /// The `#id` of the representation context. From
    /// [`of_item`](Self::of_item), the first of the agreeing contexts, in
    /// file order of the representations that hold the item.
    pub context: u64,
    /// `coordinate_space_dimension` of a geometric representation context;
    /// `None` for any other context.
    pub dimension: Option<u32>,
    #[cfg_attr(feature = "serde", serde(serialize_with = "outcome"))]
    length: Result<Unit, UnitError>,
    #[cfg_attr(feature = "serde", serde(serialize_with = "outcome"))]
    plane_angle: Result<Unit, UnitError>,
    #[cfg_attr(feature = "serde", serde(serialize_with = "outcome"))]
    solid_angle: Result<Unit, UnitError>,
    #[cfg_attr(feature = "serde", serde(serialize_with = "outcome"))]
    uncertainty: Result<Vec<Uncertainty>, UnitError>,
}

/// Serializes a per-kind result as `{"ok": value}` or `{"error": error}`.
#[cfg(feature = "serde")]
fn outcome<T, S>(result: &Result<T, UnitError>, serializer: S) -> Result<S::Ok, S::Error>
where
    T: serde::Serialize,
    S: serde::Serializer,
{
    use serde::ser::SerializeMap;
    let mut map = serializer.serialize_map(Some(1))?;
    match result {
        Ok(value) => map.serialize_entry("ok", value)?,
        Err(error) => map.serialize_entry("error", error)?,
    }
    map.end()
}

impl ContextUnits {
    /// Reads representation context `#context`.
    ///
    /// A context that assigns no units, such as the parametric 2D context
    /// of a definitional representation, reads with every kind
    /// [`Missing`](UnitError::Missing).
    ///
    /// # Errors
    ///
    /// [`UnitError::Undefined`] if there is no such instance (`from` and
    /// `to` are then both `context`), [`UnitError::NotAContext`] if it is not
    /// a representation context, and [`UnitError::Malformed`] if its
    /// `coordinate_space_dimension` is not a positive integer. Problems with
    /// its units or uncertainties are reported per kind instead.
    pub fn of_context(graph: &Graph<'_>, context: u64) -> Result<Self, UnitError> {
        let exchange = graph.exchange();
        let instance = exchange.get(context).ok_or(UnitError::Undefined {
            from: context,
            to: context,
        })?;
        let records: Vec<Record<'_>> = exchange.records(instance).collect();
        if !records.iter().any(|r| record::is_context_record(*r)) {
            return Err(UnitError::NotAContext {
                instance: context,
                entity: record::entity(exchange, instance),
            });
        }
        // A simple instance's record also holds representation_context's
        // (context_identifier, context_type) first; a partial record of a
        // complex instance holds only its own attribute.
        let simple = !instance.is_complex();
        let own = |name: &'static str| -> Option<Result<Param<'_>, UnitError>> {
            let found = records.iter().copied().find(|r| r.is(name))?;
            Some(
                record::exact_params(found, if simple { 3 } else { 1 })
                    .and_then(|params| params.into_iter().last())
                    .ok_or(UnitError::Malformed {
                        instance: context,
                        reason: match name {
                            GEOMETRIC => "GEOMETRIC_REPRESENTATION_CONTEXT has the wrong number of attributes",
                            UNITS => "GLOBAL_UNIT_ASSIGNED_CONTEXT has the wrong number of attributes",
                            _ => "GLOBAL_UNCERTAINTY_ASSIGNED_CONTEXT has the wrong number of attributes",
                        },
                    }),
            )
        };

        let dimension = match own(GEOMETRIC).transpose()? {
            None => None,
            Some(Param::Integer(literal)) => Some(
                literal
                    .to_i64()
                    .and_then(|value| u32::try_from(value).ok())
                    .filter(|&value| value > 0)
                    .ok_or(UnitError::Malformed {
                        instance: context,
                        reason: "coordinate_space_dimension is not a positive integer",
                    })?,
            ),
            Some(_) => {
                return Err(UnitError::Malformed {
                    instance: context,
                    reason: "coordinate_space_dimension is not a positive integer",
                });
            }
        };

        let units = match own(UNITS) {
            None => Ok(Vec::new()),
            Some(param) => param.and_then(|param| {
                reference_set(&param).ok_or(UnitError::Malformed {
                    instance: context,
                    reason: "the unit set is not a non-empty set of references",
                })
            }),
        };
        let [length, plane_angle, solid_angle] = match units {
            Ok(units) => context_units(graph, context, &units),
            Err(error) => [error.clone(), error.clone(), error].map(Err),
        };

        let uncertainty = match own(UNCERTAINTIES) {
            None => Ok(Vec::new()),
            Some(param) => param
                .and_then(|param| {
                    reference_set(&param).ok_or(UnitError::Malformed {
                        instance: context,
                        reason: "the uncertainty set is not a non-empty set of references",
                    })
                })
                .and_then(|ids| {
                    ids.into_iter()
                        .map(|id| read_uncertainty(graph, id, context))
                        .collect()
                }),
        };

        Ok(Self {
            context,
            dimension,
            length,
            plane_angle,
            solid_angle,
            uncertainty,
        })
    }

    /// Reads the `context_of_items` of representation `#representation`.
    ///
    /// # Errors
    ///
    /// As [`representation_context`], then as [`of_context`](Self::of_context).
    pub fn of_representation(graph: &Graph<'_>, representation: u64) -> Result<Self, UnitError> {
        Self::of_context(graph, representation_context(graph, representation)?)
    }

    /// Reads the units of `#item` from the context of every representation
    /// that lists it, directly or through the shells, solids and surface
    /// models that contain it (`closed_shell` and `open_shell` faces, an
    /// `oriented_closed_shell`'s shell, a solid's outer shell and voids, a
    /// surface model's shells or faces).
    ///
    /// Every such representation must be readable, and they must agree: a
    /// kind is `Ok` only if every holding context reads it with a
    /// bit-identical factor, and the uncertainties only if every context
    /// states the same names and values. Contexts that disagree on a kind
    /// make that kind [`ContextsDisagree`](UnitError::ContextsDisagree);
    /// a kind one of them cannot read carries that context's error.
    ///
    /// # Errors
    ///
    /// [`UnitError::Undefined`] if there is no such instance,
    /// [`UnitError::NotHeld`] if no representation holds it through a
    /// container stepq knows, [`UnitError::HolderUnreadable`] if a
    /// representation that lists it, or its context, cannot be read,
    /// [`UnitError::Malformed`] if a shell, solid or model that refers to it
    /// has the wrong number of attributes, and
    /// [`UnitError::ContextsDisagree`] with `kind: None` if the holding
    /// contexts differ in dimension.
    pub fn of_item(graph: &Graph<'_>, item: u64) -> Result<Self, UnitError> {
        let mut contexts: Vec<Self> = Vec::new();
        for (holder, context) in holders(graph, item)? {
            if contexts.iter().any(|units| units.context == context) {
                continue;
            }
            let units =
                Self::of_context(graph, context).map_err(|reason| UnitError::HolderUnreadable {
                    item,
                    holder,
                    reason: Box::new(reason),
                })?;
            contexts.push(units);
        }
        let mut contexts = contexts.into_iter();
        let Some(first) = contexts.next() else {
            return Err(UnitError::NotHeld { item });
        };
        let others: Vec<Self> = contexts.collect();
        if let Some(other) = others.iter().find(|o| o.dimension != first.dimension) {
            return Err(UnitError::ContextsDisagree {
                item,
                kind: None,
                contexts: [first.context, other.context],
            });
        }
        let length = agree(item, &first, &others, UnitKind::Length);
        let plane_angle = agree(item, &first, &others, UnitKind::PlaneAngle);
        let solid_angle = agree(item, &first, &others, UnitKind::SolidAngle);
        let uncertainty = agree_uncertainty(item, &first, &others);
        Ok(Self {
            length,
            plane_angle,
            solid_angle,
            uncertainty,
            ..first
        })
    }

    /// The length unit; its factor converts to millimetres.
    ///
    /// # Errors
    ///
    /// Why the context's length unit cannot be read: it states none
    /// ([`Missing`](UnitError::Missing)), several that differ
    /// ([`Ambiguous`](UnitError::Ambiguous)), or one that cannot be read
    /// ([`Unreadable`](UnitError::Unreadable)), among others.
    pub fn length(&self) -> Result<&Unit, UnitError> {
        self.unit(UnitKind::Length)
    }

    /// The plane angle unit; its factor converts to radians.
    ///
    /// # Errors
    ///
    /// As [`length`](Self::length), for the plane angle unit.
    pub fn plane_angle(&self) -> Result<&Unit, UnitError> {
        self.unit(UnitKind::PlaneAngle)
    }

    /// The solid angle unit; its factor converts to steradians.
    ///
    /// # Errors
    ///
    /// As [`length`](Self::length), for the solid angle unit.
    pub fn solid_angle(&self) -> Result<&Unit, UnitError> {
        self.unit(UnitKind::SolidAngle)
    }

    /// The unit of `kind`.
    ///
    /// # Errors
    ///
    /// As [`length`](Self::length), for that kind.
    pub fn unit(&self, kind: UnitKind) -> Result<&Unit, UnitError> {
        self.result(kind).as_ref().map_err(Clone::clone)
    }

    /// Every uncertainty the context states, in order; empty only if it
    /// states none.
    ///
    /// # Errors
    ///
    /// If a stated uncertainty cannot be read, or the set that lists them is
    /// malformed: a stated tolerance is never dropped.
    pub fn uncertainty(&self) -> Result<&[Uncertainty], UnitError> {
        self.uncertainty.as_deref().map_err(Clone::clone)
    }

    fn result(&self, kind: UnitKind) -> &Result<Unit, UnitError> {
        match kind {
            UnitKind::Length => &self.length,
            UnitKind::PlaneAngle => &self.plane_angle,
            UnitKind::SolidAngle => &self.solid_angle,
        }
    }
}

const GEOMETRIC: &str = "GEOMETRIC_REPRESENTATION_CONTEXT";
const UNITS: &str = "GLOBAL_UNIT_ASSIGNED_CONTEXT";
const UNCERTAINTIES: &str = "GLOBAL_UNCERTAINTY_ASSIGNED_CONTEXT";

/// Reads named unit `#id`: an SI unit or a conversion-based unit of length,
/// plane angle or solid angle, simple or complex. This also resolves the
/// unit `#id` of a [`props::Value`](crate::props::Value) or of a PMI value.
///
/// The kind comes from the unit's `LENGTH_UNIT`, `PLANE_ANGLE_UNIT` or
/// `SOLID_ANGLE_UNIT` record, or failing that from the SI unit's name or
/// the base unit. Every other statement of a kind must agree with it: the
/// SI name, the base unit, the measure type of the conversion factor
/// (`LENGTH_MEASURE_WITH_UNIT`, …), the type of its value
/// (`LENGTH_MEASURE(25.4)`, …) and explicit dimensional exponents. A plain
/// `MEASURE_WITH_UNIT`, a `RATIO_MEASURE` and an untyped number name no
/// kind.
///
/// # Errors
///
/// A [`UnitError`] naming the innermost instance that cannot be read: see
/// its variants.
pub fn unit(graph: &Graph<'_>, id: u64) -> Result<Unit, UnitError> {
    unit_at(graph, id, id, 0)
}

/// The `context_of_items` of representation `#representation`, checked: the
/// instance must be a representation (a simple instance read from its first
/// three attributes, a complex one from its `REPRESENTATION` partial, with a
/// string name, a list of references and a reference), and the instance it
/// names must be a representation context.
///
/// # Errors
///
/// [`UnitError::Undefined`] if either instance is missing,
/// [`UnitError::NotARepresentation`] or [`UnitError::NotAContext`].
pub fn representation_context(graph: &Graph<'_>, representation: u64) -> Result<u64, UnitError> {
    let exchange = graph.exchange();
    let instance = exchange.get(representation).ok_or(UnitError::Undefined {
        from: representation,
        to: representation,
    })?;
    let read = record::representation(exchange, representation).map_err(|_| {
        UnitError::NotARepresentation {
            instance: representation,
            entity: record::entity(exchange, instance),
        }
    })?;
    let context = exchange.get(read.context).ok_or(UnitError::Undefined {
        from: representation,
        to: read.context,
    })?;
    if !exchange.records(context).any(record::is_context_record) {
        return Err(UnitError::NotAContext {
            instance: read.context,
            entity: record::entity(exchange, context),
        });
    }
    Ok(read.context)
}

/// Why a unit, one kind of a context's units, its uncertainties or the
/// units of an item cannot be read as stated.
///
/// Each variant names the innermost instance at fault.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "serde", serde(tag = "kind", rename_all = "snake_case"))]
#[non_exhaustive]
pub enum UnitError {
    /// An instance refers to an `#id` that is not defined. When `to` is the
    /// `#id` the caller passed, `from` equals `to`.
    Undefined {
        /// The instance that refers to `to`.
        from: u64,
        /// The missing `#id`.
        to: u64,
    },
    /// Not a unit at all, such as a `DIMENSIONAL_EXPONENTS`.
    NotAUnit {
        /// The instance.
        instance: u64,
        /// Its upper-cased entity types, joined with `+`.
        entity: String,
    },
    /// Valid STEP that stepq does not read: a unit of another quantity
    /// (mass, time, …), a derived unit, a context-dependent unit (which
    /// states no factor), an SI unit such as the gram, or a named unit that
    /// is neither SI nor conversion-based.
    Unsupported {
        /// The unit.
        instance: u64,
        /// Its upper-cased entity types, joined with `+`.
        entity: String,
    },
    /// Contradicts the schema: a wrong number of attributes, an attribute of
    /// the wrong type, an unknown SI prefix, an SI unit that is also
    /// conversion-based, or a unit or uncertainty set that is `$`, empty or
    /// lists something other than references.
    Malformed {
        /// The instance.
        instance: u64,
        /// What is wrong.
        reason: &'static str,
    },
    /// A conversion factor or uncertainty that is not a positive finite
    /// number once converted: zero, negative or too large.
    NotPositive {
        /// The measure with unit that states it.
        instance: u64,
        /// The value after conversion.
        value: f64,
    },
    /// The unit's statements of its kind disagree: its kind record, its SI
    /// name, its base unit, its factor's measure type, its factor's value
    /// type or its dimensional exponents. All-zero exponents, which every
    /// angle has, are reported as a plane angle.
    KindMismatch {
        /// The unit, or the uncertainty whose value type contradicts its
        /// unit.
        instance: u64,
        /// The kind it is read as.
        declared: UnitKind,
        /// The kind another statement names.
        found: UnitKind,
    },
    /// A chain of conversion-based units more than 8 deep, or a cycle.
    TooDeep {
        /// The unit at which the chain was cut.
        instance: u64,
    },
    /// Not a representation context.
    NotAContext {
        /// The instance.
        instance: u64,
        /// Its upper-cased entity types, joined with `+`.
        entity: String,
    },
    /// Not a representation, or not one with a string name, a list of
    /// item references and a context reference.
    NotARepresentation {
        /// The instance.
        instance: u64,
        /// Its upper-cased entity types, joined with `+`.
        entity: String,
    },
    /// The context assigns no unit of this kind.
    Missing {
        /// The context.
        context: u64,
        /// The kind.
        #[cfg_attr(feature = "serde", serde(rename = "unit_kind"))]
        kind: UnitKind,
    },
    /// The context assigns several units of this kind whose factors differ.
    Ambiguous {
        /// The context.
        context: u64,
        /// The kind.
        #[cfg_attr(feature = "serde", serde(rename = "unit_kind"))]
        kind: UnitKind,
        /// Every unit of this kind it assigns, in order.
        units: Vec<u64>,
    },
    /// A unit the context assigns cannot be read, and it is of this kind or
    /// its kind cannot be told (it may be the length unit). Another readable
    /// unit of the same kind does not make up for it.
    Unreadable {
        /// The context.
        context: u64,
        /// The unit.
        unit: u64,
        /// Why it cannot be read.
        reason: Box<UnitError>,
    },
    /// No representation holds the item, directly or through a container
    /// stepq knows.
    NotHeld {
        /// The item.
        item: u64,
    },
    /// A representation that lists the item, or that representation's
    /// context, cannot be read. The item is not read from the others.
    HolderUnreadable {
        /// The item.
        item: u64,
        /// The representation.
        holder: u64,
        /// Why it cannot be read.
        reason: Box<UnitError>,
    },
    /// The item is held in contexts that disagree.
    ContextsDisagree {
        /// The item.
        item: u64,
        /// The kind whose units (or, from
        /// [`ContextUnits::uncertainty`], whose uncertainties) differ;
        /// `None` if the coordinate space dimensions differ.
        #[cfg_attr(feature = "serde", serde(rename = "unit_kind"))]
        kind: Option<UnitKind>,
        /// Two contexts that disagree.
        contexts: [u64; 2],
    },
}

impl fmt::Display for UnitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Undefined { from, to } if from == to => write!(f, "#{to} is not defined"),
            Self::Undefined { from, to } => {
                write!(f, "#{from} refers to #{to}, which is not defined")
            }
            Self::NotAUnit { instance, entity } => {
                write!(f, "#{instance} ({entity}) is not a unit")
            }
            Self::Unsupported { instance, entity } => write!(
                f,
                "#{instance} ({entity}) is not read: only SI and conversion-based units of \
                 length, plane angle and solid angle are"
            ),
            Self::Malformed { instance, reason } => write!(f, "#{instance}: {reason}"),
            Self::NotPositive { instance, value } => {
                write!(f, "#{instance}: {value} is not a positive finite number")
            }
            Self::KindMismatch {
                instance,
                declared,
                found,
            } => write!(
                f,
                "#{instance} is a {declared} unit, but its definition states a {found}"
            ),
            Self::TooDeep { instance } => write!(
                f,
                "#{instance}: conversion-based units nested more than {MAX_CONVERSION_DEPTH} \
                 deep, or in a cycle"
            ),
            Self::NotAContext { instance, entity } => {
                write!(f, "#{instance} ({entity}) is not a representation context")
            }
            Self::NotARepresentation { instance, entity } => {
                write!(f, "#{instance} ({entity}) is not a representation")
            }
            Self::Missing { context, kind } => {
                write!(f, "context #{context} assigns no {kind} unit")
            }
            Self::Ambiguous {
                context,
                kind,
                units,
            } => {
                write!(f, "context #{context} assigns {kind} units that differ:")?;
                for (i, unit) in units.iter().enumerate() {
                    write!(f, "{} #{unit}", if i == 0 { "" } else { "," })?;
                }
                Ok(())
            }
            Self::Unreadable {
                context,
                unit,
                reason,
            } => write!(
                f,
                "context #{context}: unit #{unit} cannot be read: {reason}"
            ),
            Self::NotHeld { item } => write!(
                f,
                "#{item} is held by no representation, directly or through a shell, solid or \
                 surface model"
            ),
            Self::HolderUnreadable {
                item,
                holder,
                reason,
            } => write!(
                f,
                "#{item} is held by representation #{holder}, which cannot be read: {reason}"
            ),
            Self::ContextsDisagree {
                item,
                kind,
                contexts: [a, b],
            } => match kind {
                Some(kind) => write!(
                    f,
                    "#{item} is held in contexts #{a} and #{b}, which state different {kind} \
                     units or uncertainties"
                ),
                None => write!(
                    f,
                    "#{item} is held in contexts #{a} and #{b}, which differ in dimension"
                ),
            },
        }
    }
}

/// A context's unit of each kind, read or refused, from the units it lists.
fn context_units(graph: &Graph<'_>, context: u64, units: &[u64]) -> [Result<Unit, UnitError>; 3] {
    let mut found: [Vec<Unit>; 3] = Default::default();
    let mut refused: [Option<(u64, UnitError)>; 3] = Default::default();
    let mut unknown: Option<(u64, UnitError)> = None;
    for &id in units {
        match unit_at(graph, id, context, 0) {
            Ok(unit) => found[unit.kind.slot()].push(unit),
            Err(error) => match declared_kind(graph.exchange(), id) {
                Declared::Kind(kind) => {
                    refused[kind.slot()].get_or_insert((id, error));
                }
                Declared::Other => {}
                Declared::Unknown => {
                    unknown.get_or_insert((id, error));
                }
            },
        }
    }
    UnitKind::ALL.map(|kind| {
        let slot = kind.slot();
        if let Some((unit, error)) = refused[slot].as_ref().or(unknown.as_ref()) {
            return Err(UnitError::Unreadable {
                context,
                unit: *unit,
                reason: Box::new(error.clone()),
            });
        }
        let units = &found[slot];
        let first = units.first().ok_or(UnitError::Missing { context, kind })?;
        if units
            .iter()
            .any(|unit| unit.factor.to_bits() != first.factor.to_bits())
        {
            return Err(UnitError::Ambiguous {
                context,
                kind,
                units: units.iter().map(|unit| unit.instance).collect(),
            });
        }
        Ok(first.clone())
    })
}

/// The merged unit of `kind` for an item held in `first` and `others`.
fn agree(
    item: u64,
    first: &ContextUnits,
    others: &[ContextUnits],
    kind: UnitKind,
) -> Result<Unit, UnitError> {
    let unit = first.result(kind).clone()?;
    for other in others {
        let theirs = other.result(kind).as_ref().map_err(Clone::clone)?;
        if theirs.factor.to_bits() != unit.factor.to_bits() {
            return Err(UnitError::ContextsDisagree {
                item,
                kind: Some(kind),
                contexts: [first.context, other.context],
            });
        }
    }
    Ok(unit)
}

/// The merged uncertainties for an item held in `first` and `others`: the
/// same names, kinds and values in the same order.
fn agree_uncertainty(
    item: u64,
    first: &ContextUnits,
    others: &[ContextUnits],
) -> Result<Vec<Uncertainty>, UnitError> {
    let ours = first.uncertainty.clone()?;
    for other in others {
        let theirs = other.uncertainty.as_ref().map_err(Clone::clone)?;
        let same = |a: &Uncertainty, b: &Uncertainty| {
            a.name == b.name && a.unit.kind == b.unit.kind && a.value.to_bits() == b.value.to_bits()
        };
        let differs =
            (0..ours.len().max(theirs.len())).find(|&i| match (ours.get(i), theirs.get(i)) {
                (Some(a), Some(b)) => !same(a, b),
                _ => true,
            });
        if let Some(i) = differs {
            let kind = ours.get(i).or(theirs.get(i)).map(|u| u.unit.kind);
            return Err(UnitError::ContextsDisagree {
                item,
                kind,
                contexts: [first.context, other.context],
            });
        }
    }
    Ok(ours)
}

/// The representations that hold `#item` and their contexts, in file order:
/// every representation that lists it or a container holding it.
fn holders(graph: &Graph<'_>, item: u64) -> Result<Vec<(u64, u64)>, UnitError> {
    let exchange = graph.exchange();
    let start = graph.node(item).ok_or(UnitError::Undefined {
        from: item,
        to: item,
    })?;
    let mut seen = HashSet::from([start]);
    let mut stack = vec![start];
    let mut found: Vec<(usize, u64)> = Vec::new();
    while let Some(node) = stack.pop() {
        let held = graph.instance(node).id;
        for &user in graph.referenced_by(node) {
            match holding(graph, item, graph.instance(user), held)? {
                Holding::Container => {
                    if seen.insert(user) {
                        stack.push(user);
                    }
                }
                Holding::Representation(context) => found.push((user, context)),
                Holding::No => {}
            }
        }
    }
    found.sort_unstable();
    found.dedup();
    Ok(found
        .into_iter()
        .map(|(node, context)| (exchange.instances()[node].id, context))
        .collect())
}

enum Holding {
    /// A shell, solid or model that holds it: look further up.
    Container,
    /// A representation that lists it, with its context.
    Representation(u64),
    /// Refers to it some other way.
    No,
}

/// How `user` holds `#held`, on the way up from `#item`.
fn holding(graph: &Graph<'_>, item: u64, user: &Instance, held: u64) -> Result<Holding, UnitError> {
    let exchange = graph.exchange();
    let unreadable = |reason| UnitError::HolderUnreadable {
        item,
        holder: user.id,
        reason: Box::new(reason),
    };
    if user.is_complex() {
        // Containers are only read as simple instances; a complex
        // representation keeps its attributes in its REPRESENTATION partial.
        if record::partial(exchange, user, "REPRESENTATION").is_none() {
            return Ok(Holding::No);
        }
        let Ok(read) = record::representation(exchange, user.id) else {
            return Err(unreadable(UnitError::NotARepresentation {
                instance: user.id,
                entity: record::entity(exchange, user),
            }));
        };
        if !read.items.contains(&held) {
            return Ok(Holding::No);
        }
        return representation_context(graph, user.id)
            .map(Holding::Representation)
            .map_err(unreadable);
    }
    let Some(simple) = exchange.records(user).next() else {
        return Ok(Holding::No);
    };
    if let Some(container) = record::container(simple) {
        let params =
            record::exact_params(simple, container.params).ok_or(UnitError::Malformed {
                instance: user.id,
                reason: "a shell, solid or surface model with the wrong number of attributes",
            })?;
        let holds = container
            .holds
            .iter()
            .any(|&index| record::holds(&params[index], held));
        return Ok(if holds {
            Holding::Container
        } else {
            Holding::No
        });
    }
    // A representation lists its items in its second attribute and names its
    // context in its third.
    let lists = simple
        .param(1)
        .and_then(|param| param.list())
        .is_some_and(|mut items| items.any(|p| p.reference() == Some(held)));
    if !lists {
        return Ok(Holding::No);
    }
    let names_context = match simple.param(2) {
        Some(Param::Reference(context)) => exchange
            .get(context)
            .is_none_or(|context| exchange.records(context).any(record::is_context_record)),
        _ => false,
    };
    if !(names_context || record::upper(simple.name()).contains("REPRESENTATION")) {
        // A set or curve listing it, which is not a representation.
        return Ok(Holding::No);
    }
    representation_context(graph, user.id)
        .map(Holding::Representation)
        .map_err(unreadable)
}

/// A non-empty list of references; `None` for `$`, an empty list or a list
/// with anything else in it.
fn reference_set(param: &Param<'_>) -> Option<Vec<u64>> {
    let ids = param
        .list()?
        .map(|item| item.reference())
        .collect::<Option<Vec<u64>>>()?;
    (!ids.is_empty()).then_some(ids)
}

/// What kind a unit declares without reading its factor, to tell which of a
/// context's kinds an unreadable unit refuses.
enum Declared {
    Kind(UnitKind),
    /// A unit of another quantity, which a context's kinds ignore.
    Other,
    /// Cannot be told: it may be any kind.
    Unknown,
}

fn declared_kind(exchange: &Exchange<'_>, id: u64) -> Declared {
    let Some(instance) = exchange.get(id) else {
        return Declared::Unknown;
    };
    let records: Vec<Record<'_>> = exchange.records(instance).collect();
    match kind_record(&records) {
        Ok(Some(kind)) => return Declared::Kind(kind),
        Ok(None) => {}
        Err(_) => return Declared::Unknown,
    }
    if records.iter().any(|r| is_other_quantity(*r)) {
        return Declared::Other;
    }
    if let Some(si) = records.iter().find(|r| r.is("SI_UNIT")) {
        let name = si
            .params()
            .last()
            .and_then(|p| p.literal())
            .and_then(Literal::enumeration);
        return match name.and_then(si_kind) {
            Some(kind) => Declared::Kind(kind),
            None if name.is_some() => Declared::Other,
            None => Declared::Unknown,
        };
    }
    Declared::Unknown
}

/// The kind a `LENGTH_UNIT`, `PLANE_ANGLE_UNIT` or `SOLID_ANGLE_UNIT` record
/// declares, if any; an error if records declare two kinds.
fn kind_record(records: &[Record<'_>]) -> Result<Option<UnitKind>, &'static str> {
    let mut kind = None;
    for record in records {
        let this = if record.is("LENGTH_UNIT") {
            UnitKind::Length
        } else if record.is("PLANE_ANGLE_UNIT") {
            UnitKind::PlaneAngle
        } else if record.is("SOLID_ANGLE_UNIT") {
            UnitKind::SolidAngle
        } else {
            continue;
        };
        if kind.is_some_and(|k| k != this) {
            return Err("declared as two kinds of unit");
        }
        kind = Some(this);
    }
    Ok(kind)
}

/// A unit record of another quantity: `MASS_UNIT`, `AREA_UNIT`,
/// `DERIVED_UNIT`, … — any `*_UNIT` but the named, SI, conversion-based and
/// context-dependent units and the three kinds read here.
fn is_other_quantity(record: Record<'_>) -> bool {
    const READ: &[&str] = &[
        "NAMED_UNIT",
        "SI_UNIT",
        "CONVERSION_BASED_UNIT",
        "CONTEXT_DEPENDENT_UNIT",
        "LENGTH_UNIT",
        "PLANE_ANGLE_UNIT",
        "SOLID_ANGLE_UNIT",
    ];
    record::upper(record.name()).ends_with("_UNIT") && !READ.iter().any(|name| record.is(name))
}

fn si_kind(name: &str) -> Option<UnitKind> {
    match name.to_ascii_uppercase().as_str() {
        "METRE" => Some(UnitKind::Length),
        "RADIAN" => Some(UnitKind::PlaneAngle),
        "STERADIAN" => Some(UnitKind::SolidAngle),
        _ => None,
    }
}

/// The factor of an SI prefix: to the millimetre for a prefixed metre, and
/// as is for the radian and steradian. Each entry is the nearest double to
/// the exact power of ten, so a millimetre is exactly 1 and a centimetre
/// exactly 10.
fn si_factor(kind: UnitKind, prefix: Option<&str>) -> Option<f64> {
    let Some(prefix) = prefix else {
        return Some(if kind == UnitKind::Length { 1e3 } else { 1.0 });
    };
    let (metre, other) = match prefix.to_ascii_uppercase().as_str() {
        "EXA" => (1e21, 1e18),
        "PETA" => (1e18, 1e15),
        "TERA" => (1e15, 1e12),
        "GIGA" => (1e12, 1e9),
        "MEGA" => (1e9, 1e6),
        "KILO" => (1e6, 1e3),
        "HECTO" => (1e5, 1e2),
        "DECA" => (1e4, 1e1),
        "DECI" => (1e2, 1e-1),
        "CENTI" => (1e1, 1e-2),
        "MILLI" => (1.0, 1e-3),
        "MICRO" => (1e-3, 1e-6),
        "NANO" => (1e-6, 1e-9),
        "PICO" => (1e-9, 1e-12),
        "FEMTO" => (1e-12, 1e-15),
        "ATTO" => (1e-15, 1e-18),
        _ => return None,
    };
    Some(if kind == UnitKind::Length {
        metre
    } else {
        other
    })
}

/// What a statement says about a unit's kind.
#[derive(Clone, Copy)]
enum Evidence {
    Kind(UnitKind),
    /// All-zero dimensional exponents: a plane or a solid angle.
    Angle,
}

/// Fails if `evidence` names another kind than `kind`.
fn check(instance: u64, kind: UnitKind, evidence: Option<Evidence>) -> Result<(), UnitError> {
    let found = match evidence {
        Some(Evidence::Kind(found)) if found != kind => found,
        Some(Evidence::Angle) if kind == UnitKind::Length => UnitKind::PlaneAngle,
        _ => return Ok(()),
    };
    Err(UnitError::KindMismatch {
        instance,
        declared: kind,
        found,
    })
}

fn unit_at(graph: &Graph<'_>, id: u64, from: u64, depth: usize) -> Result<Unit, UnitError> {
    if depth > MAX_CONVERSION_DEPTH {
        return Err(UnitError::TooDeep { instance: id });
    }
    let exchange = graph.exchange();
    let instance = exchange
        .get(id)
        .ok_or(UnitError::Undefined { from, to: id })?;
    let records: Vec<Record<'_>> = exchange.records(instance).collect();
    let declared = kind_record(&records).map_err(|reason| UnitError::Malformed {
        instance: id,
        reason,
    })?;
    let unsupported = || UnitError::Unsupported {
        instance: id,
        entity: record::entity(exchange, instance),
    };
    if records
        .iter()
        .any(|r| r.is("CONTEXT_DEPENDENT_UNIT") || is_other_quantity(*r))
    {
        return Err(unsupported());
    }
    // A complex instance states named_unit.dimensions in its NAMED_UNIT
    // partial; a simple SI or conversion-based unit as its first attribute.
    let dimensions = if instance.is_complex() {
        match records.iter().find(|r| r.is("NAMED_UNIT")) {
            Some(named) => Some(
                record::exact_params(*named, 1)
                    .and_then(|params| params.into_iter().next())
                    .ok_or(UnitError::Malformed {
                        instance: id,
                        reason: "NAMED_UNIT has the wrong number of attributes",
                    })?,
            ),
            None => None,
        }
    } else {
        None
    };
    let simple = !instance.is_complex();
    let si = records.iter().find(|r| r.is("SI_UNIT"));
    let conversion = records.iter().find(|r| r.is("CONVERSION_BASED_UNIT"));
    let unit = Read {
        graph,
        id,
        declared,
    };
    match (si, conversion) {
        (Some(_), Some(_)) => Err(UnitError::Malformed {
            instance: id,
            reason: "both an SI unit and a conversion-based unit",
        }),
        (Some(&si), None) => unit.si(si, simple, dimensions),
        (None, Some(&conversion)) => unit.conversion(conversion, simple, dimensions, depth),
        (None, None) if declared.is_some() || records.iter().any(|r| r.is("NAMED_UNIT")) => {
            Err(unsupported())
        }
        (None, None) => Err(UnitError::NotAUnit {
            instance: id,
            entity: record::entity(exchange, instance),
        }),
    }
}

/// One named unit being read.
struct Read<'g, 'a> {
    graph: &'g Graph<'a>,
    id: u64,
    /// The kind its kind record declares, if any.
    declared: Option<UnitKind>,
}

impl Read<'_, '_> {
    fn malformed(&self, reason: &'static str) -> UnitError {
        UnitError::Malformed {
            instance: self.id,
            reason,
        }
    }

    /// `SI_UNIT(prefix, name)` in a complex instance, `SI_UNIT(*, prefix,
    /// name)` as a simple one.
    fn si(
        &self,
        si: Record<'_>,
        simple: bool,
        dimensions: Option<Param<'_>>,
    ) -> Result<Unit, UnitError> {
        let params = record::exact_params(si, if simple { 3 } else { 2 })
            .ok_or_else(|| self.malformed("SI_UNIT has the wrong number of attributes"))?;
        let (dimensions, prefix, name) = match params.as_slice() {
            [dimensions, prefix, name] => (Some(dimensions.clone()), prefix, name),
            [prefix, name] => (dimensions, prefix, name),
            _ => return Err(self.malformed("SI_UNIT has the wrong number of attributes")),
        };
        let prefix = match prefix {
            Param::Unset => None,
            Param::Enumeration(literal) => literal.enumeration(),
            _ => return Err(self.malformed("the SI prefix is neither $ nor an enumeration")),
        };
        let Some(name) = name.literal().and_then(Literal::enumeration) else {
            return Err(self.malformed("the SI unit name is not an enumeration"));
        };
        let Some(kind) = si_kind(name) else {
            let exchange = self.graph.exchange();
            return Err(UnitError::Unsupported {
                instance: self.id,
                entity: exchange
                    .get(self.id)
                    .map(|instance| record::entity(exchange, instance))
                    .unwrap_or_default(),
            });
        };
        if let Some(declared) = self.declared {
            check(self.id, declared, Some(Evidence::Kind(kind)))?;
        }
        if let Some(dimensions) = dimensions {
            check(self.id, kind, self.dimensions(&dimensions)?)?;
        }
        let factor = si_factor(kind, prefix).ok_or_else(|| self.malformed("unknown SI prefix"))?;
        Ok(Unit {
            instance: self.id,
            kind,
            name: format!("{}{name}", prefix.unwrap_or_default()).to_ascii_lowercase(),
            factor,
        })
    }

    /// `CONVERSION_BASED_UNIT(name, factor)` in a complex instance,
    /// `CONVERSION_BASED_UNIT(dimensions, name, factor)` as a simple one.
    fn conversion(
        &self,
        conversion: Record<'_>,
        simple: bool,
        dimensions: Option<Param<'_>>,
        depth: usize,
    ) -> Result<Unit, UnitError> {
        let params =
            record::exact_params(conversion, if simple { 3 } else { 2 }).ok_or_else(|| {
                self.malformed("CONVERSION_BASED_UNIT has the wrong number of attributes")
            })?;
        let (dimensions, name, factor) = match params.as_slice() {
            [dimensions, name, factor] => (Some(dimensions.clone()), name, factor),
            [name, factor] => (dimensions, name, factor),
            _ => {
                return Err(
                    self.malformed("CONVERSION_BASED_UNIT has the wrong number of attributes")
                );
            }
        };
        let name = string(name)
            .ok_or_else(|| self.malformed("the conversion-based unit's name is not a string"))?;
        let factor = factor
            .reference()
            .ok_or_else(|| self.malformed("the conversion factor is not a reference"))?;
        let measure = factor_measure(self.graph, factor, self.id)?;
        let base = unit_at(self.graph, measure.unit, factor, depth + 1)?;
        let kind = self.declared.unwrap_or(base.kind);
        check(self.id, kind, Some(Evidence::Kind(base.kind)))?;
        for evidence in measure.kinds {
            check(self.id, kind, Some(Evidence::Kind(evidence)))?;
        }
        if let Some(dimensions) = dimensions {
            check(self.id, kind, self.dimensions(&dimensions)?)?;
        }
        let value = measure.value * base.factor;
        if !(value.is_finite() && value > 0.0) {
            return Err(UnitError::NotPositive {
                instance: factor,
                value,
            });
        }
        Ok(Unit {
            instance: self.id,
            kind,
            name: name.to_lowercase(),
            factor: value,
        })
    }

    /// What `named_unit.dimensions` says about the kind: nothing if derived
    /// (`*`, as an SI unit writes it), else its dimensional exponents.
    fn dimensions(&self, param: &Param<'_>) -> Result<Option<Evidence>, UnitError> {
        let id = match param {
            Param::Derived => return Ok(None),
            Param::Reference(id) => *id,
            _ => {
                return Err(self.malformed("named_unit.dimensions is neither * nor a reference"));
            }
        };
        let exchange = self.graph.exchange();
        let instance = exchange.get(id).ok_or(UnitError::Undefined {
            from: self.id,
            to: id,
        })?;
        let malformed = |reason| UnitError::Malformed {
            instance: id,
            reason,
        };
        let exponents: Vec<f64> = exchange
            .records(instance)
            .next()
            .filter(|r| !instance.is_complex() && r.is("DIMENSIONAL_EXPONENTS"))
            .and_then(|r| record::exact_params(r, 7))
            .and_then(|params| params.iter().map(record::real).collect())
            .ok_or_else(|| malformed("not a DIMENSIONAL_EXPONENTS with seven numbers"))?;
        #[expect(
            clippy::float_cmp,
            reason = "exponents are small integers written as reals; any other value is not a length or angle"
        )]
        let is = |expected: [f64; 7]| exponents.iter().zip(expected).all(|(a, b)| *a == b);
        if is([1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]) {
            Ok(Some(Evidence::Kind(UnitKind::Length)))
        } else if is([0.0; 7]) {
            Ok(Some(Evidence::Angle))
        } else {
            Err(malformed(
                "the dimensional exponents are those of neither a length nor an angle",
            ))
        }
    }
}

/// A conversion factor: its value, its unit and the kinds its types state.
struct Measure {
    value: f64,
    unit: u64,
    kinds: Vec<UnitKind>,
}

/// Reads `#id`, the conversion factor of unit `#from`: a simple
/// `*MEASURE_WITH_UNIT(value, unit)`, or the `MEASURE_WITH_UNIT` partial of
/// a complex instance.
fn factor_measure(graph: &Graph<'_>, id: u64, from: u64) -> Result<Measure, UnitError> {
    let exchange = graph.exchange();
    let instance = exchange
        .get(id)
        .ok_or(UnitError::Undefined { from, to: id })?;
    let malformed = |reason| UnitError::Malformed {
        instance: id,
        reason,
    };
    let records: Vec<Record<'_>> = exchange.records(instance).collect();
    let mut kinds = Vec::new();
    let mut base = None;
    for partial in &records {
        let name = record::upper(partial.name());
        if !name.ends_with("MEASURE_WITH_UNIT") {
            continue;
        }
        // A simple instance is its own measure with unit; a complex one
        // holds value and unit in its MEASURE_WITH_UNIT partial.
        if !instance.is_complex() || name == "MEASURE_WITH_UNIT" {
            base = Some(*partial);
        }
        match name.as_str() {
            "MEASURE_WITH_UNIT" | "RATIO_MEASURE_WITH_UNIT" => {}
            "LENGTH_MEASURE_WITH_UNIT" => kinds.push(UnitKind::Length),
            "PLANE_ANGLE_MEASURE_WITH_UNIT" => kinds.push(UnitKind::PlaneAngle),
            "SOLID_ANGLE_MEASURE_WITH_UNIT" => kinds.push(UnitKind::SolidAngle),
            _ => {
                return Err(malformed(
                    "the conversion factor measures something other than a length or an angle",
                ));
            }
        }
    }
    let params = base
        .and_then(|partial| record::exact_params(partial, 2))
        .ok_or_else(|| malformed("not a measure with unit with a value and a unit"))?;
    let (value, typed) =
        measure_value(&params[0]).ok_or_else(|| malformed("the value is not a number"))?;
    if let Some(kind) = typed.map_err(malformed)? {
        kinds.push(kind);
    }
    let unit = params[1]
        .reference()
        .ok_or_else(|| malformed("the unit is not a reference"))?;
    Ok(Measure { value, unit, kinds })
}

/// The kind a typed value's type names, if any, or why it names none that
/// fits.
type TypedKind = Result<Option<UnitKind>, &'static str>;

/// A measure's value, typed (`LENGTH_MEASURE(25.4)`) or plain, and the kind
/// its type names. `None` if it is not a number; the number may be
/// infinite, for the caller to refuse.
fn measure_value(param: &Param<'_>) -> Option<(f64, TypedKind)> {
    let (literal, kind) = match param {
        Param::Typed(typed) => {
            let [value] = record::exact_params(*typed, 1)?.try_into().ok()?;
            let kind = match record::upper(typed.name()).as_str() {
                "LENGTH_MEASURE" | "POSITIVE_LENGTH_MEASURE" => Ok(Some(UnitKind::Length)),
                "PLANE_ANGLE_MEASURE" | "POSITIVE_PLANE_ANGLE_MEASURE" => {
                    Ok(Some(UnitKind::PlaneAngle))
                }
                "SOLID_ANGLE_MEASURE" => Ok(Some(UnitKind::SolidAngle)),
                "RATIO_MEASURE" | "POSITIVE_RATIO_MEASURE" => Ok(None),
                _ => Err("the value is typed as a measure of another quantity"),
            };
            (value, kind)
        }
        other => (other.clone(), Ok(None)),
    };
    match literal {
        Param::Integer(literal) | Param::Real(literal) => Some((literal.to_f64()?, kind)),
        _ => None,
    }
}

/// A string literal, decoded.
fn string(param: &Param<'_>) -> Option<String> {
    match param {
        Param::String(literal) => literal.decode().ok().map(std::borrow::Cow::into_owned),
        _ => None,
    }
}

/// Reads `#id`, listed in context `#from`'s uncertainties:
/// `UNCERTAINTY_MEASURE_WITH_UNIT(value, unit, name, description)`, or the
/// `MEASURE_WITH_UNIT` and `UNCERTAINTY_MEASURE_WITH_UNIT` partials of a
/// complex instance.
fn read_uncertainty(graph: &Graph<'_>, id: u64, from: u64) -> Result<Uncertainty, UnitError> {
    let exchange = graph.exchange();
    let instance = exchange
        .get(id)
        .ok_or(UnitError::Undefined { from, to: id })?;
    let malformed = |reason| UnitError::Malformed {
        instance: id,
        reason,
    };
    let params: Vec<Param<'_>> = if instance.is_complex() {
        let measure = record::partial(exchange, instance, "MEASURE_WITH_UNIT")
            .and_then(|r| record::exact_params(r, 2));
        let own = record::partial(exchange, instance, "UNCERTAINTY_MEASURE_WITH_UNIT")
            .and_then(|r| record::exact_params(r, 2));
        measure.zip(own).map(|(mut a, b)| {
            a.extend(b);
            a
        })
    } else {
        exchange
            .records(instance)
            .next()
            .filter(|r| r.is("UNCERTAINTY_MEASURE_WITH_UNIT"))
            .and_then(|r| record::exact_params(r, 4))
    }
    .ok_or_else(|| malformed("not an UNCERTAINTY_MEASURE_WITH_UNIT with four attributes"))?;
    let [value, unit, name, description] = params.as_slice() else {
        return Err(malformed(
            "not an UNCERTAINTY_MEASURE_WITH_UNIT with four attributes",
        ));
    };
    let (raw, typed) =
        measure_value(value).ok_or_else(|| malformed("the value is not a number"))?;
    let unit = unit
        .reference()
        .ok_or_else(|| malformed("the unit is not a reference"))?;
    let unit = unit_at(graph, unit, id, 0)?;
    if let Some(found) = typed.map_err(malformed)? {
        check(id, unit.kind, Some(Evidence::Kind(found)))?;
    }
    let name = string(name).ok_or_else(|| malformed("the name is not a string"))?;
    let description = match description {
        Param::Unset => None,
        other => Some(string(other).ok_or_else(|| malformed("the description is not a string"))?),
    };
    let value = raw * unit.factor;
    if !(value.is_finite() && value > 0.0) {
        return Err(UnitError::NotPositive {
            instance: id,
            value,
        });
    }
    Ok(Uncertainty {
        instance: id,
        name,
        description,
        value,
        unit,
    })
}

#[cfg(test)]
mod tests {
    use std::fmt::Write as _;

    use super::*;
    use crate::p21::parse;

    fn graph(data: &str) -> Graph<'static> {
        let src = format!(
            "ISO-10303-21;HEADER;FILE_SCHEMA(('X'));ENDSEC;DATA;{data}ENDSEC;END-ISO-10303-21;"
        );
        let src: &'static [u8] = Box::leak(src.into_bytes().into_boxed_slice());
        Graph::build(parse(src).unwrap())
    }

    /// Units shared by the tests: millimetre, radian, steradian, inches
    /// over the millimetre and the centimetre, a degree, a foot over the
    /// inch through a plain measure with unit, a simple micrometre.
    const BASE: &str = "#1=(LENGTH_UNIT()NAMED_UNIT(*)SI_UNIT(.MILLI.,.METRE.));\
        #2=(NAMED_UNIT(*)PLANE_ANGLE_UNIT()SI_UNIT($,.RADIAN.));\
        #3=(NAMED_UNIT(*)SI_UNIT($,.STERADIAN.)SOLID_ANGLE_UNIT());\
        #4=DIMENSIONAL_EXPONENTS(1.,0.,0.,0.,0.,0.,0.);\
        #5=LENGTH_MEASURE_WITH_UNIT(LENGTH_MEASURE(25.4),#1);\
        #6=(CONVERSION_BASED_UNIT('INCH',#5)LENGTH_UNIT()NAMED_UNIT(#4));\
        #7=(LENGTH_UNIT()NAMED_UNIT(*)SI_UNIT(.CENTI.,.METRE.));\
        #8=LENGTH_MEASURE_WITH_UNIT(LENGTH_MEASURE(2.54),#7);\
        #9=(CONVERSION_BASED_UNIT('INCH',#8)LENGTH_UNIT()NAMED_UNIT(#4));\
        #10=PLANE_ANGLE_MEASURE_WITH_UNIT(PLANE_ANGLE_MEASURE(0.0174532925),#2);\
        #11=DIMENSIONAL_EXPONENTS(0.,0.,0.,0.,0.,0.,0.);\
        #12=(CONVERSION_BASED_UNIT('DEGREE',#10)NAMED_UNIT(#11)PLANE_ANGLE_UNIT());\
        #13=MEASURE_WITH_UNIT(LENGTH_MEASURE(12.),#6);\
        #14=(CONVERSION_BASED_UNIT('FOOT',#13)LENGTH_UNIT()NAMED_UNIT(#4));\
        #15=SI_UNIT(*,.MICRO.,.METRE.);\
        #16=UNCERTAINTY_MEASURE_WITH_UNIT(LENGTH_MEASURE(1.E-03),#6,'distance_accuracy_value','');";

    /// A complex 3D context #100 assigning `units`, with uncertainty
    /// `uncertainty`, plus `extra` instances.
    fn context(units: &str, uncertainty: &str, extra: &str) -> Graph<'static> {
        graph(&format!(
            "{BASE}#100=(GEOMETRIC_REPRESENTATION_CONTEXT(3)\
             GLOBAL_UNCERTAINTY_ASSIGNED_CONTEXT(({uncertainty}))\
             GLOBAL_UNIT_ASSIGNED_CONTEXT(({units}))REPRESENTATION_CONTEXT('',''));{extra}"
        ))
    }

    fn bits(unit: Result<Unit, UnitError>) -> u64 {
        unit.unwrap().factor.to_bits()
    }

    #[test]
    fn si_prefixes_come_from_an_exact_table() {
        const PREFIXES: [(&str, i32); 16] = [
            ("EXA", 18),
            ("PETA", 15),
            ("TERA", 12),
            ("GIGA", 9),
            ("MEGA", 6),
            ("KILO", 3),
            ("HECTO", 2),
            ("DECA", 1),
            ("DECI", -1),
            ("CENTI", -2),
            ("MILLI", -3),
            ("MICRO", -6),
            ("NANO", -9),
            ("PICO", -12),
            ("FEMTO", -15),
            ("ATTO", -18),
        ];
        let mut data = String::new();
        for (i, (prefix, _)) in PREFIXES.iter().enumerate() {
            write!(
                data,
                "#{}=(LENGTH_UNIT()NAMED_UNIT(*)SI_UNIT(.{prefix}.,.METRE.));\
                 #{}=(NAMED_UNIT(*)PLANE_ANGLE_UNIT()SI_UNIT(.{prefix}.,.RADIAN.));",
                100 + i,
                200 + i
            )
            .unwrap();
        }
        let g = graph(&format!("{BASE}{data}"));
        // The nearest double to 10^n, as Rust's correctly rounded parser
        // gives it.
        let power = |n: i32| format!("1e{n}").parse::<f64>().unwrap().to_bits();
        for (i, (prefix, exponent)) in PREFIXES.iter().enumerate() {
            let id = 100 + u64::try_from(i).unwrap();
            let metre = unit(&g, id).unwrap();
            assert_eq!(metre.factor.to_bits(), power(exponent + 3), "{prefix}METRE");
            assert_eq!(metre.name, format!("{prefix}metre").to_ascii_lowercase());
            assert_eq!(bits(unit(&g, id + 100)), power(*exponent), "{prefix}RADIAN");
        }
        assert_eq!(
            bits(unit(&g, 1)),
            1.0f64.to_bits(),
            "a millimetre is exactly 1"
        );
        assert_eq!(bits(unit(&g, 7)), 10.0f64.to_bits());
        assert_eq!(bits(unit(&g, 2)), 1.0f64.to_bits());
        assert_eq!(bits(unit(&g, 3)), 1.0f64.to_bits());
        let micrometre = unit(&g, 15).unwrap();
        assert_eq!(
            (micrometre.kind, micrometre.name.as_str()),
            (UnitKind::Length, "micrometre"),
            "a simple SI unit takes its kind from its name"
        );
        assert_eq!(micrometre.factor.to_bits(), 1e-3f64.to_bits());
    }

    #[test]
    fn conversion_based_units_multiply_their_stated_factors() {
        let g = graph(BASE);
        let inch = unit(&g, 6).unwrap();
        assert_eq!(
            (inch.instance, inch.kind, inch.name.as_str()),
            (6, UnitKind::Length, "inch")
        );
        assert_eq!(inch.factor.to_bits(), 25.4f64.to_bits());
        assert_eq!(
            bits(unit(&g, 9)),
            (2.54 * 10.0f64).to_bits(),
            "an inch of 2.54 centimetres is the f64 product"
        );
        let foot = unit(&g, 14).unwrap();
        assert_eq!(foot.name, "foot");
        assert_eq!(
            foot.factor.to_bits(),
            (12.0 * 25.4f64).to_bits(),
            "a foot of 12 inches, through a plain MEASURE_WITH_UNIT"
        );
        let degree = unit(&g, 12).unwrap();
        assert_eq!(
            (degree.kind, degree.name.as_str()),
            (UnitKind::PlaneAngle, "degree")
        );
        assert_eq!(
            degree.factor.to_bits(),
            0.017_453_292_5f64.to_bits(),
            "used as written, never snapped to pi/180"
        );
    }

    #[test]
    fn kinds_come_from_the_base_and_neutral_types_are_accepted() {
        let g = graph(&format!(
            "{BASE}#20=MEASURE_WITH_UNIT(2.54,#7);\
             #21=(CONVERSION_BASED_UNIT('inch',#20)NAMED_UNIT(#4));\
             #22=MEASURE_WITH_UNIT(RATIO_MEASURE(0.0174532925),#2);\
             #23=(CONVERSION_BASED_UNIT('degree',#22)NAMED_UNIT(*));\
             #24=CONVERSION_BASED_UNIT(#4,'INCH',#5);"
        ));
        let inch = unit(&g, 21).unwrap();
        assert_eq!((inch.kind, inch.name.as_str()), (UnitKind::Length, "inch"));
        assert_eq!(inch.factor.to_bits(), (2.54 * 10.0f64).to_bits());
        assert_eq!(unit(&g, 23).unwrap().kind, UnitKind::PlaneAngle);
        let simple = unit(&g, 24).unwrap();
        assert_eq!(
            (simple.kind, simple.factor.to_bits()),
            (UnitKind::Length, 25.4f64.to_bits())
        );
    }

    #[test]
    fn names_are_labelled_as_info_labels_them() {
        let g = graph(BASE);
        let info = crate::info::Info::new(&g);
        for (id, labels) in [
            (1, &info.units.length),
            (6, &info.units.length),
            (7, &info.units.length),
            (14, &info.units.length),
            (2, &info.units.plane_angle),
            (12, &info.units.plane_angle),
            (3, &info.units.solid_angle),
        ] {
            let name = unit(&g, id).unwrap().name;
            assert!(labels.contains(&name), "#{id}: {name} not in {labels:?}");
        }
    }

    #[test]
    fn units_that_cannot_be_read_say_why() {
        let g = graph(&format!(
            "{BASE}#20=(CONTEXT_DEPENDENT_UNIT('PIXEL')LENGTH_UNIT()NAMED_UNIT(#4));\
             #21=(MASS_UNIT()NAMED_UNIT(*)SI_UNIT(.KILO.,.GRAM.));\
             #22=SI_UNIT(*,$,.SECOND.);\
             #23=DERIVED_UNIT((#24));#24=DERIVED_UNIT_ELEMENT(#1,2.);\
             #25=MASS_MEASURE_WITH_UNIT(MASS_MEASURE(1000.),#21);\
             #26=(CONVERSION_BASED_UNIT('ton',#25)MASS_UNIT()NAMED_UNIT(#4));\
             #40=(CONVERSION_BASED_UNIT('X',#998)LENGTH_UNIT()NAMED_UNIT(#4));\
             #41=LENGTH_MEASURE_WITH_UNIT(LENGTH_MEASURE(1.),#997);\
             #42=(CONVERSION_BASED_UNIT('X',#41)LENGTH_UNIT()NAMED_UNIT(#4));\
             #43=(NAMED_UNIT(#4)LENGTH_UNIT());"
        ));
        let unsupported = |instance, entity: &str| {
            Err(UnitError::Unsupported {
                instance,
                entity: entity.to_owned(),
            })
        };
        assert_eq!(
            unit(&g, 999),
            Err(UnitError::Undefined { from: 999, to: 999 })
        );
        assert_eq!(
            unit(&g, 40),
            Err(UnitError::Undefined { from: 40, to: 998 })
        );
        assert_eq!(
            unit(&g, 42),
            Err(UnitError::Undefined { from: 41, to: 997 })
        );
        assert_eq!(
            unit(&g, 4),
            Err(UnitError::NotAUnit {
                instance: 4,
                entity: "DIMENSIONAL_EXPONENTS".to_owned()
            })
        );
        assert_eq!(
            unit(&g, 20),
            unsupported(20, "CONTEXT_DEPENDENT_UNIT+LENGTH_UNIT+NAMED_UNIT")
        );
        assert_eq!(
            unit(&g, 21),
            unsupported(21, "MASS_UNIT+NAMED_UNIT+SI_UNIT")
        );
        assert_eq!(unit(&g, 22), unsupported(22, "SI_UNIT"), "an SI second");
        assert_eq!(unit(&g, 23), unsupported(23, "DERIVED_UNIT"));
        assert_eq!(
            unit(&g, 26),
            unsupported(26, "CONVERSION_BASED_UNIT+MASS_UNIT+NAMED_UNIT")
        );
        assert_eq!(
            unit(&g, 43),
            unsupported(43, "NAMED_UNIT+LENGTH_UNIT"),
            "a named unit that states no factor"
        );
    }

    #[test]
    fn malformed_units_say_what_is_wrong() {
        let g = graph(&format!(
            "{BASE}#27=(LENGTH_UNIT()NAMED_UNIT(*)SI_UNIT(.QUECTO.,.METRE.));\
             #28=(CONVERSION_BASED_UNIT('X',#5)LENGTH_UNIT()NAMED_UNIT(*)SI_UNIT(.MILLI.,.METRE.));\
             #29=(LENGTH_UNIT()NAMED_UNIT(*)PLANE_ANGLE_UNIT()SI_UNIT(.MILLI.,.METRE.));\
             #30=(LENGTH_UNIT()NAMED_UNIT(*)SI_UNIT(.MILLI.,.METRE.,$));\
             #31=(CONVERSION_BASED_UNIT(#5,#5)LENGTH_UNIT()NAMED_UNIT(#4));\
             #32=(CONVERSION_BASED_UNIT('X',25.4)LENGTH_UNIT()NAMED_UNIT(#4));\
             #33=LENGTH_MEASURE_WITH_UNIT(LENGTH_MEASURE(25.4),#1,$);\
             #34=(CONVERSION_BASED_UNIT('X',#33)LENGTH_UNIT()NAMED_UNIT(#4));\
             #35=LENGTH_MEASURE_WITH_UNIT(LENGTH_MEASURE('x'),#1);\
             #36=(CONVERSION_BASED_UNIT('X',#35)LENGTH_UNIT()NAMED_UNIT(#4));\
             #37=(CONVERSION_BASED_UNIT('X',#5)LENGTH_UNIT()NAMED_UNIT($));\
             #38=DIMENSIONAL_EXPONENTS(0.,1.,0.,0.,0.,0.,0.);\
             #39=(CONVERSION_BASED_UNIT('X',#5)LENGTH_UNIT()NAMED_UNIT(#38));\
             #44=TIME_MEASURE_WITH_UNIT(TIME_MEASURE(1.),#1);\
             #45=(CONVERSION_BASED_UNIT('X',#44)LENGTH_UNIT()NAMED_UNIT(#4));\
             #46=LENGTH_MEASURE_WITH_UNIT(COUNT_MEASURE(1.),#1);\
             #47=(CONVERSION_BASED_UNIT('X',#46)LENGTH_UNIT()NAMED_UNIT(#4));"
        ));
        let malformed = |instance, reason| Err(UnitError::Malformed { instance, reason });
        assert_eq!(unit(&g, 27), malformed(27, "unknown SI prefix"));
        assert_eq!(
            unit(&g, 28),
            malformed(28, "both an SI unit and a conversion-based unit")
        );
        assert_eq!(unit(&g, 29), malformed(29, "declared as two kinds of unit"));
        assert_eq!(
            unit(&g, 30),
            malformed(30, "SI_UNIT has the wrong number of attributes")
        );
        assert_eq!(
            unit(&g, 31),
            malformed(31, "the conversion-based unit's name is not a string")
        );
        assert_eq!(
            unit(&g, 32),
            malformed(32, "the conversion factor is not a reference")
        );
        assert_eq!(
            unit(&g, 34),
            malformed(33, "not a measure with unit with a value and a unit")
        );
        assert_eq!(unit(&g, 36), malformed(35, "the value is not a number"));
        assert_eq!(
            unit(&g, 37),
            malformed(37, "named_unit.dimensions is neither * nor a reference")
        );
        assert_eq!(
            unit(&g, 39),
            malformed(
                38,
                "the dimensional exponents are those of neither a length nor an angle"
            )
        );
        assert_eq!(
            unit(&g, 45),
            malformed(
                44,
                "the conversion factor measures something other than a length or an angle"
            )
        );
        assert_eq!(
            unit(&g, 47),
            malformed(46, "the value is typed as a measure of another quantity")
        );
    }

    #[test]
    fn factors_must_be_positive_and_finite() {
        let g = graph(&format!(
            "{BASE}#20=LENGTH_MEASURE_WITH_UNIT(LENGTH_MEASURE(0.),#1);\
             #21=(CONVERSION_BASED_UNIT('ZERO',#20)LENGTH_UNIT()NAMED_UNIT(#4));\
             #22=LENGTH_MEASURE_WITH_UNIT(LENGTH_MEASURE(-25.4),#1);\
             #23=(CONVERSION_BASED_UNIT('NEGATIVE',#22)LENGTH_UNIT()NAMED_UNIT(#4));\
             #24=LENGTH_MEASURE_WITH_UNIT(LENGTH_MEASURE(1.E999),#1);\
             #25=(CONVERSION_BASED_UNIT('HUGE',#24)LENGTH_UNIT()NAMED_UNIT(#4));"
        ));
        assert_eq!(
            unit(&g, 21),
            Err(UnitError::NotPositive {
                instance: 20,
                value: 0.0
            })
        );
        assert_eq!(
            unit(&g, 23),
            Err(UnitError::NotPositive {
                instance: 22,
                value: -25.4
            })
        );
        assert_eq!(
            unit(&g, 25),
            Err(UnitError::NotPositive {
                instance: 24,
                value: f64::INFINITY
            })
        );
    }

    #[test]
    fn every_statement_of_a_kind_must_agree() {
        use UnitKind::{Length, PlaneAngle};
        let g = graph(&format!(
            "{BASE}#20=(CONVERSION_BASED_UNIT('BAD',#10)LENGTH_UNIT()NAMED_UNIT(#4));\
             #21=(LENGTH_UNIT()NAMED_UNIT(*)SI_UNIT($,.RADIAN.));\
             #22=PLANE_ANGLE_MEASURE_WITH_UNIT(PLANE_ANGLE_MEASURE(25.4),#1);\
             #23=(CONVERSION_BASED_UNIT('X',#22)NAMED_UNIT(*));\
             #24=LENGTH_MEASURE_WITH_UNIT(PLANE_ANGLE_MEASURE(25.4),#1);\
             #25=(CONVERSION_BASED_UNIT('X',#24)LENGTH_UNIT()NAMED_UNIT(#4));\
             #26=(CONVERSION_BASED_UNIT('X',#5)LENGTH_UNIT()NAMED_UNIT(#11));\
             #27=(CONVERSION_BASED_UNIT('X',#10)NAMED_UNIT(#4)PLANE_ANGLE_UNIT());\
             #28=(LENGTH_UNIT()NAMED_UNIT(#11)SI_UNIT(.MILLI.,.METRE.));\
             #29=(LENGTH_MEASURE_WITH_UNIT()MEASURE_WITH_UNIT(PLANE_ANGLE_MEASURE(1.),#2));\
             #30=(CONVERSION_BASED_UNIT('X',#29)NAMED_UNIT(*));"
        ));
        let mismatch = |instance, declared, found| {
            Err(UnitError::KindMismatch {
                instance,
                declared,
                found,
            })
        };
        assert_eq!(
            unit(&g, 20),
            mismatch(20, Length, PlaneAngle),
            "a length in degrees"
        );
        assert_eq!(
            unit(&g, 21),
            mismatch(21, Length, PlaneAngle),
            "a length in SI radians"
        );
        assert_eq!(
            unit(&g, 23),
            mismatch(23, Length, PlaneAngle),
            "an angle measure over a millimetre"
        );
        assert_eq!(
            unit(&g, 25),
            mismatch(25, Length, PlaneAngle),
            "a plane angle value as a length"
        );
        assert_eq!(
            unit(&g, 26),
            mismatch(26, Length, PlaneAngle),
            "a length with an angle's exponents"
        );
        assert_eq!(
            unit(&g, 27),
            mismatch(27, PlaneAngle, Length),
            "an angle with a length's exponents"
        );
        assert_eq!(
            unit(&g, 28),
            mismatch(28, Length, PlaneAngle),
            "SI exponents"
        );
        assert_eq!(
            unit(&g, 30),
            mismatch(30, PlaneAngle, Length),
            "a complex measure's subtype partial"
        );
    }

    #[test]
    fn conversion_chains_are_bounded() {
        // A chain of `n` conversion-based units over the millimetre.
        let chain = |n: u64| {
            let mut links = String::new();
            for i in 0..n {
                let (unit, measure, base) = (
                    100 + 2 * i,
                    101 + 2 * i,
                    if i == 0 { 1 } else { 98 + 2 * i },
                );
                write!(
                    links,
                    "#{measure}=LENGTH_MEASURE_WITH_UNIT(LENGTH_MEASURE(2.),#{base});\
                     #{unit}=(CONVERSION_BASED_UNIT('X',#{measure})LENGTH_UNIT()NAMED_UNIT(*));"
                )
                .unwrap();
            }
            let data = format!("#1=(LENGTH_UNIT()NAMED_UNIT(*)SI_UNIT(.MILLI.,.METRE.));{links}");
            (graph(&data), 98 + 2 * n)
        };
        let (g, top) = chain(8);
        assert_eq!(
            bits(unit(&g, top)),
            256.0f64.to_bits(),
            "8 conversions are read"
        );
        let (g, top) = chain(9);
        assert!(
            matches!(unit(&g, top), Err(UnitError::TooDeep { .. })),
            "9 are not"
        );
        let g = graph(&format!(
            "{BASE}#20=(CONVERSION_BASED_UNIT('LOOP',#21)LENGTH_UNIT()NAMED_UNIT(#4));\
             #21=LENGTH_MEASURE_WITH_UNIT(LENGTH_MEASURE(2.),#20);"
        ));
        assert!(
            matches!(unit(&g, 20), Err(UnitError::TooDeep { .. })),
            "a cycle"
        );
    }

    #[test]
    fn a_context_reads_each_kind_on_its_own() {
        let g = context("#6,#12,#3", "#16", "");
        let units = ContextUnits::of_context(&g, 100).unwrap();
        assert_eq!((units.context, units.dimension), (100, Some(3)));
        assert_eq!(units.length().unwrap().instance, 6);
        assert_eq!(units.plane_angle().unwrap().instance, 12);
        assert_eq!(units.solid_angle().unwrap().instance, 3);
        let [tolerance] = units.uncertainty().unwrap() else {
            panic!("one uncertainty");
        };
        assert_eq!(
            (
                tolerance.instance,
                tolerance.name.as_str(),
                tolerance.description.as_deref()
            ),
            (16, "distance_accuracy_value", Some(""))
        );
        assert_eq!(
            tolerance.value.to_bits(),
            (1e-3 * 25.4f64).to_bits(),
            "converted with its own unit, the inch"
        );

        let g = context("#6,#12", "#16", "");
        let units = ContextUnits::of_context(&g, 100).unwrap();
        assert_eq!(
            units.solid_angle(),
            Err(UnitError::Missing {
                context: 100,
                kind: UnitKind::SolidAngle
            })
        );
        assert!(
            units.length().is_ok(),
            "a missing solid angle unit blocks nothing else"
        );

        let g = context("#1,#6,#12", "#16", "");
        let units = ContextUnits::of_context(&g, 100).unwrap();
        assert_eq!(
            units.length(),
            Err(UnitError::Ambiguous {
                context: 100,
                kind: UnitKind::Length,
                units: vec![1, 6]
            })
        );
        assert!(units.plane_angle().is_ok());

        let g = context(
            "#1,#20,#12",
            "#16",
            "#20=(LENGTH_UNIT()NAMED_UNIT(*)SI_UNIT(.MILLI.,.METRE.));",
        );
        let units = ContextUnits::of_context(&g, 100).unwrap();
        assert_eq!(
            units.length().unwrap().instance,
            1,
            "units of one kind with bit-equal factors agree"
        );
    }

    #[test]
    fn a_context_refuses_rather_than_assumes() {
        let extra = "#20=(CONTEXT_DEPENDENT_UNIT('PIXEL')LENGTH_UNIT()NAMED_UNIT(#4));\
                     #21=(MASS_UNIT()NAMED_UNIT(*)SI_UNIT(.KILO.,.GRAM.));";
        let g = context("#1,#20,#12", "#16", extra);
        let units = ContextUnits::of_context(&g, 100).unwrap();
        assert!(
            matches!(
                units.length(),
                Err(UnitError::Unreadable {
                    context: 100,
                    unit: 20,
                    ..
                })
            ),
            "an unreadable length unit is not made up for by a readable one: {:?}",
            units.length()
        );
        assert!(units.plane_angle().is_ok());

        let g = context("#1,#21,#12", "#16", extra);
        let units = ContextUnits::of_context(&g, 100).unwrap();
        assert_eq!(
            units.length().unwrap().instance,
            1,
            "a mass unit is not a length unit"
        );

        // A unit whose kind cannot be told may be the length unit.
        for (listed, reason) in [
            (
                "#1,#4,#12",
                UnitError::NotAUnit {
                    instance: 4,
                    entity: "DIMENSIONAL_EXPONENTS".to_owned(),
                },
            ),
            ("#1,#999,#12", UnitError::Undefined { from: 100, to: 999 }),
        ] {
            let g = context(listed, "#16", "");
            let units = ContextUnits::of_context(&g, 100).unwrap();
            let unit = if listed.contains("#4") { 4 } else { 999 };
            for kind in UnitKind::ALL {
                assert_eq!(
                    units.unit(kind),
                    Err(UnitError::Unreadable {
                        context: 100,
                        unit,
                        reason: Box::new(reason.clone())
                    }),
                    "{listed}"
                );
            }
        }
    }

    #[test]
    fn malformed_sets_are_never_empty() {
        for units in ["$", "", "#1,'x'"] {
            let g = graph(&format!(
                "{BASE}#100=(GEOMETRIC_REPRESENTATION_CONTEXT(3)\
                 GLOBAL_UNIT_ASSIGNED_CONTEXT({}) REPRESENTATION_CONTEXT('',''));",
                if units == "$" {
                    "$".to_owned()
                } else {
                    format!("({units})")
                }
            ));
            let read = ContextUnits::of_context(&g, 100).unwrap();
            for kind in UnitKind::ALL {
                assert_eq!(
                    read.unit(kind),
                    Err(UnitError::Malformed {
                        instance: 100,
                        reason: "the unit set is not a non-empty set of references"
                    }),
                    "{units:?}"
                );
            }
            assert_eq!(read.uncertainty(), Ok(&[][..]), "none stated");
        }
        // The review's case: an uncertainty set of `($)` is malformed, not
        // empty, and does not block the units.
        let g = context("#1,#2", "$", "");
        let units = ContextUnits::of_context(&g, 100).unwrap();
        assert_eq!(
            units.uncertainty(),
            Err(UnitError::Malformed {
                instance: 100,
                reason: "the uncertainty set is not a non-empty set of references"
            })
        );
        assert!(units.length().is_ok());
    }

    #[test]
    fn uncertainties_use_their_own_unit() {
        let g = context(
            "#6,#12",
            "#20,#21",
            "#20=UNCERTAINTY_MEASURE_WITH_UNIT(LENGTH_MEASURE(0.01),#1,'closure',$);\
             #21=UNCERTAINTY_MEASURE_WITH_UNIT(PLANE_ANGLE_MEASURE(0.5),#12,'angle','x');",
        );
        let units = ContextUnits::of_context(&g, 100).unwrap();
        let [mm, angle] = units.uncertainty().unwrap() else {
            panic!("two uncertainties");
        };
        assert_eq!(
            (mm.value.to_bits(), mm.description.as_deref()),
            (0.01f64.to_bits(), None),
            "in millimetres although the context is in inches"
        );
        assert_eq!(angle.value.to_bits(), (0.5 * 0.017_453_292_5f64).to_bits());
        assert_eq!(angle.unit.kind, UnitKind::PlaneAngle);

        for (uncertainty, expected) in [
            (
                "#20=UNCERTAINTY_MEASURE_WITH_UNIT(LENGTH_MEASURE(-1.),#1,'closure','');",
                UnitError::NotPositive {
                    instance: 20,
                    value: -1.0,
                },
            ),
            (
                "#20=UNCERTAINTY_MEASURE_WITH_UNIT(PLANE_ANGLE_MEASURE(1.),#1,'closure','');",
                UnitError::KindMismatch {
                    instance: 20,
                    declared: UnitKind::Length,
                    found: UnitKind::PlaneAngle,
                },
            ),
            (
                "#20=UNCERTAINTY_MEASURE_WITH_UNIT(LENGTH_MEASURE(1.),#1,$,'');",
                UnitError::Malformed {
                    instance: 20,
                    reason: "the name is not a string",
                },
            ),
            (
                "#20=LENGTH_MEASURE_WITH_UNIT(LENGTH_MEASURE(1.),#1);",
                UnitError::Malformed {
                    instance: 20,
                    reason: "not an UNCERTAINTY_MEASURE_WITH_UNIT with four attributes",
                },
            ),
        ] {
            let g = context("#1,#12", "#20", uncertainty);
            let units = ContextUnits::of_context(&g, 100).unwrap();
            assert_eq!(units.uncertainty(), Err(expected));
            assert!(units.length().is_ok(), "an uncertainty blocks no unit");
        }

        let g = graph(&format!(
            "{BASE}#100=(GEOMETRIC_REPRESENTATION_CONTEXT(3)\
             GLOBAL_UNIT_ASSIGNED_CONTEXT((#1,#2))REPRESENTATION_CONTEXT('',''));"
        ));
        assert_eq!(
            ContextUnits::of_context(&g, 100).unwrap().uncertainty(),
            Ok(&[][..]),
            "empty only if none is stated"
        );
    }

    #[test]
    fn contexts_simple_parametric_and_malformed() {
        let g = graph(&format!(
            "{BASE}#100=GLOBAL_UNIT_ASSIGNED_CONTEXT('','',(#1,#2));\
             #101=(GEOMETRIC_REPRESENTATION_CONTEXT(2)PARAMETRIC_REPRESENTATION_CONTEXT()\
             REPRESENTATION_CONTEXT('2D SPACE',''));\
             #102=(GEOMETRIC_REPRESENTATION_CONTEXT(0)REPRESENTATION_CONTEXT('',''));\
             #103=(GEOMETRIC_REPRESENTATION_CONTEXT(3.)REPRESENTATION_CONTEXT('',''));\
             #104=GEOMETRIC_REPRESENTATION_CONTEXT('','',3);\
             #105=(GEOMETRIC_REPRESENTATION_CONTEXT(3)GLOBAL_UNIT_ASSIGNED_CONTEXT((#1),$)\
             REPRESENTATION_CONTEXT('',''));\
             #106=SHAPE_REPRESENTATION('',(),#101);"
        ));
        let simple = ContextUnits::of_context(&g, 100).unwrap();
        assert_eq!(simple.length().unwrap().instance, 1);
        assert_eq!(simple.dimension, None);

        let parametric = ContextUnits::of_context(&g, 101).unwrap();
        assert_eq!(parametric.dimension, Some(2));
        assert_eq!(
            parametric.length(),
            Err(UnitError::Missing {
                context: 101,
                kind: UnitKind::Length
            })
        );
        assert_eq!(
            ContextUnits::of_context(&g, 104).unwrap().dimension,
            Some(3),
            "a simple geometric context"
        );
        for id in [102, 103] {
            assert_eq!(
                ContextUnits::of_context(&g, id),
                Err(UnitError::Malformed {
                    instance: id,
                    reason: "coordinate_space_dimension is not a positive integer"
                })
            );
        }
        assert_eq!(
            ContextUnits::of_context(&g, 105).unwrap().length(),
            Err(UnitError::Malformed {
                instance: 105,
                reason: "GLOBAL_UNIT_ASSIGNED_CONTEXT has the wrong number of attributes"
            })
        );
        assert_eq!(
            ContextUnits::of_context(&g, 106),
            Err(UnitError::NotAContext {
                instance: 106,
                entity: "SHAPE_REPRESENTATION".to_owned()
            })
        );
        assert_eq!(
            ContextUnits::of_context(&g, 999),
            Err(UnitError::Undefined { from: 999, to: 999 })
        );
    }

    #[test]
    fn representations_name_their_context() {
        let g = context(
            "#1,#2",
            "#16",
            "#200=ADVANCED_BREP_SHAPE_REPRESENTATION('',(#201),#100);\
             #201=AXIS2_PLACEMENT_3D('',#202,$,$);#202=CARTESIAN_POINT('',(0.,0.,0.));\
             #210=(DRAUGHTING_MODEL()REPRESENTATION('',(#201),#100)SHAPE_REPRESENTATION());\
             #220=SHAPE_DEFINITION_REPRESENTATION(#221,#200);\
             #230=SHAPE_REPRESENTATION('',(#201),#555);\
             #240=SHAPE_REPRESENTATION('',(#201),#202);",
        );
        assert_eq!(representation_context(&g, 200), Ok(100));
        assert_eq!(representation_context(&g, 210), Ok(100));
        assert_eq!(
            ContextUnits::of_representation(&g, 210)
                .unwrap()
                .length()
                .unwrap()
                .instance,
            1
        );
        assert_eq!(
            representation_context(&g, 220),
            Err(UnitError::NotARepresentation {
                instance: 220,
                entity: "SHAPE_DEFINITION_REPRESENTATION".to_owned()
            })
        );
        assert_eq!(
            representation_context(&g, 230),
            Err(UnitError::Undefined { from: 230, to: 555 })
        );
        assert_eq!(
            representation_context(&g, 240),
            Err(UnitError::NotAContext {
                instance: 202,
                entity: "CARTESIAN_POINT".to_owned()
            })
        );
    }

    /// A solid #201 with one face #203 in shell #202, held by
    /// representation #200 in context #100, plus `extra`.
    fn solid(units: &str, extra: &str) -> Graph<'static> {
        context(
            units,
            "#16",
            &format!(
                "#200=ADVANCED_BREP_SHAPE_REPRESENTATION('',(#201,#205),#100);\
                 #201=MANIFOLD_SOLID_BREP('',#202);#202=CLOSED_SHELL('',(#203));\
                 #203=ADVANCED_FACE('',(),#204,.T.);#204=PLANE('',#205);\
                 #205=AXIS2_PLACEMENT_3D('',#206,$,$);#206=CARTESIAN_POINT('',(0.,0.,0.));\
                 #101=(GEOMETRIC_REPRESENTATION_CONTEXT(3)GLOBAL_UNCERTAINTY_ASSIGNED_CONTEXT((#16))\
                 GLOBAL_UNIT_ASSIGNED_CONTEXT((#1,#12))REPRESENTATION_CONTEXT('',''));\
                 #102=(GEOMETRIC_REPRESENTATION_CONTEXT(2)GLOBAL_UNCERTAINTY_ASSIGNED_CONTEXT((#16))\
                 GLOBAL_UNIT_ASSIGNED_CONTEXT((#6,#12))REPRESENTATION_CONTEXT('',''));\
                 {extra}"
            ),
        )
    }

    #[test]
    fn items_find_their_context_through_their_containers() {
        let g = solid("#6,#12", "#300=ADVANCED_FACE('',(),#204,.T.);");
        for item in [201, 202, 203, 205] {
            let units = ContextUnits::of_item(&g, item);
            assert_eq!(
                units.map(|u| (u.context, u.length().unwrap().factor.to_bits())),
                Ok((100, 25.4f64.to_bits())),
                "#{item}"
            );
        }
        assert_eq!(
            ContextUnits::of_item(&g, 300),
            Err(UnitError::NotHeld { item: 300 }),
            "an orphan face"
        );
        assert_eq!(
            ContextUnits::of_item(&g, 200),
            Err(UnitError::NotHeld { item: 200 }),
            "a representation is not an item: read it with of_representation"
        );
        assert_eq!(
            ContextUnits::of_item(&g, 204),
            Err(UnitError::NotHeld { item: 204 }),
            "a surface is not followed up through its face"
        );
        assert_eq!(
            ContextUnits::of_item(&g, 999),
            Err(UnitError::Undefined { from: 999, to: 999 })
        );
    }

    #[test]
    fn void_faces_and_surface_models_are_held() {
        let g = context(
            "#6,#12",
            "#16",
            "#200=ADVANCED_BREP_SHAPE_REPRESENTATION('',(#201),#100);\
             #201=BREP_WITH_VOIDS('',#202,(#210));#202=CLOSED_SHELL('',(#203));\
             #203=ADVANCED_FACE('',(),#999,.T.);\
             #210=ORIENTED_CLOSED_SHELL('',*,#211,.F.);#211=CLOSED_SHELL('',(#212));\
             #212=ADVANCED_FACE('',(),#999,.T.);\
             #300=MANIFOLD_SURFACE_SHAPE_REPRESENTATION('',(#301),#100);\
             #301=SHELL_BASED_SURFACE_MODEL('',(#302));#302=OPEN_SHELL('',(#303));\
             #303=ADVANCED_FACE('',(),#999,.T.);",
        );
        for item in [212, 211, 210, 303, 302, 301] {
            assert_eq!(
                ContextUnits::of_item(&g, item).map(|u| u.context),
                Ok(100),
                "#{item}"
            );
        }
    }

    #[test]
    fn every_holder_must_be_readable_and_agree() {
        // The review's cases. An inch holder that is a subtype with a
        // fourth attribute is read, not skipped.
        let g = solid(
            "#1,#12",
            "#210=TESSELLATED_SHAPE_REPRESENTATION_WITH_ACCURACY_PARAMETERS('',(#201),#103,(0.1));\
             #103=(GEOMETRIC_REPRESENTATION_CONTEXT(3)GLOBAL_UNCERTAINTY_ASSIGNED_CONTEXT((#16))\
             GLOBAL_UNIT_ASSIGNED_CONTEXT((#6,#12))REPRESENTATION_CONTEXT('',''));",
        );
        let units = ContextUnits::of_item(&g, 203).unwrap();
        assert_eq!(
            units.length(),
            Err(UnitError::ContextsDisagree {
                item: 203,
                kind: Some(UnitKind::Length),
                contexts: [100, 103]
            })
        );
        assert!(
            units.plane_angle().is_ok(),
            "the kinds that agree still read"
        );

        // A holder with a dangling context fails the item.
        let g = solid("#1,#12", "#210=SHAPE_REPRESENTATION('',(#201),#555);");
        assert_eq!(
            ContextUnits::of_item(&g, 203),
            Err(UnitError::HolderUnreadable {
                item: 203,
                holder: 210,
                reason: Box::new(UnitError::Undefined { from: 210, to: 555 })
            })
        );

        // So does a holder whose context cannot be read.
        let g = solid(
            "#1,#12",
            "#210=SHAPE_REPRESENTATION('',(#201),#104);\
             #104=(GEOMETRIC_REPRESENTATION_CONTEXT(0)REPRESENTATION_CONTEXT('',''));",
        );
        assert!(matches!(
            ContextUnits::of_item(&g, 203),
            Err(UnitError::HolderUnreadable { holder: 210, .. })
        ));

        // And a complex holder whose REPRESENTATION partial is malformed.
        let g = solid(
            "#1,#12",
            "#210=(REPRESENTATION('',(#201),#100,$)SHAPE_REPRESENTATION());",
        );
        assert!(matches!(
            ContextUnits::of_item(&g, 203),
            Err(UnitError::HolderUnreadable { holder: 210, .. })
        ));
    }

    #[test]
    fn holders_agree_kind_by_kind() {
        // A 2D and a 3D holder disagree on the dimension.
        let g = solid("#1,#12", "#210=SHAPE_REPRESENTATION('',(#201),#102);");
        assert_eq!(
            ContextUnits::of_item(&g, 203),
            Err(UnitError::ContextsDisagree {
                item: 203,
                kind: None,
                contexts: [100, 102]
            })
        );

        // Two holders whose contexts read alike agree, even as distinct
        // instances, and a malformed uncertainty in both leaves the units.
        let g = solid("#1,#12", "#210=SHAPE_REPRESENTATION('',(#201),#101);");
        let units = ContextUnits::of_item(&g, 203).unwrap();
        assert_eq!(units.context, 100);
        assert_eq!(units.length().unwrap().instance, 1);
        let g = graph(&format!(
            "{BASE}#21=UNCERTAINTY_MEASURE_WITH_UNIT(LENGTH_MEASURE(-1.),#1,'closure','');\
             #100=(GEOMETRIC_REPRESENTATION_CONTEXT(3)GLOBAL_UNCERTAINTY_ASSIGNED_CONTEXT((#21))\
             GLOBAL_UNIT_ASSIGNED_CONTEXT((#1,#2))REPRESENTATION_CONTEXT('',''));\
             #101=(GEOMETRIC_REPRESENTATION_CONTEXT(3)GLOBAL_UNCERTAINTY_ASSIGNED_CONTEXT((#21))\
             GLOBAL_UNIT_ASSIGNED_CONTEXT((#1,#2))REPRESENTATION_CONTEXT('',''));\
             #200=ADVANCED_BREP_SHAPE_REPRESENTATION('',(#201),#100);\
             #210=SHAPE_REPRESENTATION('',(#201),#101);\
             #201=MANIFOLD_SOLID_BREP('',#202);#202=CLOSED_SHELL('',(#203));\
             #203=ADVANCED_FACE('',(),#999,.T.);"
        ));
        let units = ContextUnits::of_item(&g, 203).unwrap();
        assert_eq!(units.length().unwrap().factor.to_bits(), 1.0f64.to_bits());
        assert_eq!(
            units.uncertainty(),
            Err(UnitError::NotPositive {
                instance: 21,
                value: -1.0
            })
        );

        // Uncertainties that differ disagree; a kind one holder lacks
        // carries that holder's reason.
        let g = solid(
            "#1,#12",
            "#210=SHAPE_REPRESENTATION('',(#201),#103);\
             #103=(GEOMETRIC_REPRESENTATION_CONTEXT(3)GLOBAL_UNCERTAINTY_ASSIGNED_CONTEXT((#22))\
             GLOBAL_UNIT_ASSIGNED_CONTEXT((#1))REPRESENTATION_CONTEXT('',''));\
             #22=UNCERTAINTY_MEASURE_WITH_UNIT(LENGTH_MEASURE(0.1),#1,'closure','');",
        );
        let units = ContextUnits::of_item(&g, 203).unwrap();
        assert!(units.length().is_ok());
        assert_eq!(
            units.uncertainty(),
            Err(UnitError::ContextsDisagree {
                item: 203,
                kind: Some(UnitKind::Length),
                contexts: [100, 103]
            })
        );
        assert_eq!(
            units.plane_angle(),
            Err(UnitError::Missing {
                context: 103,
                kind: UnitKind::PlaneAngle
            })
        );
    }

    #[test]
    fn malformed_containers_and_sets_that_are_not_representations() {
        let g = solid("#1,#12", "#210=CLOSED_SHELL('',(#203),$);");
        assert_eq!(
            ContextUnits::of_item(&g, 203),
            Err(UnitError::Malformed {
                instance: 210,
                reason: "a shell, solid or surface model with the wrong number of attributes"
            })
        );
        // A geometric set lists the face but is no representation; it holds
        // nothing the walk follows.
        let g = solid("#1,#12", "#210=GEOMETRIC_SET('',(#203));");
        assert_eq!(ContextUnits::of_item(&g, 203).map(|u| u.context), Ok(100));
    }

    #[test]
    fn messages_name_the_instances() {
        assert_eq!(
            UnitError::Undefined { from: 5, to: 5 }.to_string(),
            "#5 is not defined"
        );
        assert_eq!(
            UnitError::Undefined { from: 7, to: 5 }.to_string(),
            "#7 refers to #5, which is not defined"
        );
        assert_eq!(
            UnitError::Ambiguous {
                context: 1,
                kind: UnitKind::Length,
                units: vec![2, 3]
            }
            .to_string(),
            "context #1 assigns length units that differ: #2, #3"
        );
        let wrapped = crate::Error::from(UnitError::Missing {
            context: 4,
            kind: UnitKind::SolidAngle,
        });
        assert_eq!(
            wrapped.to_string(),
            "context #4 assigns no solid angle unit"
        );
    }

    /// Every serialized result type has unique keys: a tagged enum's `kind`
    /// tag must not meet a field called `kind`.
    #[cfg(feature = "serde")]
    mod serialized {
        use super::*;
        use serde::Deserialize;
        use serde::de::{self, MapAccess, SeqAccess, Visitor};

        /// Any JSON value; deserializing fails on a repeated key.
        struct Unique;

        impl<'de> Deserialize<'de> for Unique {
            fn deserialize<D: de::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                d.deserialize_any(UniqueVisitor)
            }
        }

        struct UniqueVisitor;

        impl<'de> Visitor<'de> for UniqueVisitor {
            type Value = Unique;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("JSON")
            }
            fn visit_bool<E>(self, _: bool) -> Result<Unique, E> {
                Ok(Unique)
            }
            fn visit_i64<E>(self, _: i64) -> Result<Unique, E> {
                Ok(Unique)
            }
            fn visit_u64<E>(self, _: u64) -> Result<Unique, E> {
                Ok(Unique)
            }
            fn visit_f64<E>(self, _: f64) -> Result<Unique, E> {
                Ok(Unique)
            }
            fn visit_str<E>(self, _: &str) -> Result<Unique, E> {
                Ok(Unique)
            }
            fn visit_unit<E>(self) -> Result<Unique, E> {
                Ok(Unique)
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Unique, A::Error> {
                while seq.next_element::<Unique>()?.is_some() {}
                Ok(Unique)
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Unique, A::Error> {
                let mut keys = HashSet::new();
                while let Some(key) = map.next_key::<String>()? {
                    if !keys.insert(key.clone()) {
                        return Err(de::Error::custom(format!("duplicate key {key:?}")));
                    }
                    map.next_value::<Unique>()?;
                }
                Ok(Unique)
            }
        }

        /// The value as JSON; fails on a repeated key.
        fn unique(value: &impl serde::Serialize) -> String {
            let json = serde_json::to_string(value).unwrap();
            if let Err(e) = serde_json::from_str::<Unique>(&json) {
                panic!("{json}: {e}");
            }
            json
        }

        /// One value of every variant.
        fn every_error() -> Vec<UnitError> {
            let entity = || "X".to_owned();
            vec![
                UnitError::Undefined { from: 1, to: 2 },
                UnitError::NotAUnit {
                    instance: 1,
                    entity: entity(),
                },
                UnitError::Unsupported {
                    instance: 1,
                    entity: entity(),
                },
                UnitError::Malformed {
                    instance: 1,
                    reason: "x",
                },
                UnitError::NotPositive {
                    instance: 1,
                    value: -1.0,
                },
                UnitError::KindMismatch {
                    instance: 1,
                    declared: UnitKind::Length,
                    found: UnitKind::PlaneAngle,
                },
                UnitError::TooDeep { instance: 1 },
                UnitError::NotAContext {
                    instance: 1,
                    entity: entity(),
                },
                UnitError::NotARepresentation {
                    instance: 1,
                    entity: entity(),
                },
                UnitError::Missing {
                    context: 1,
                    kind: UnitKind::SolidAngle,
                },
                UnitError::Ambiguous {
                    context: 1,
                    kind: UnitKind::Length,
                    units: vec![2, 3],
                },
                UnitError::Unreadable {
                    context: 1,
                    unit: 2,
                    reason: Box::new(UnitError::TooDeep { instance: 2 }),
                },
                UnitError::NotHeld { item: 1 },
                UnitError::HolderUnreadable {
                    item: 1,
                    holder: 2,
                    reason: Box::new(UnitError::Undefined { from: 2, to: 3 }),
                },
                UnitError::ContextsDisagree {
                    item: 1,
                    kind: Some(UnitKind::Length),
                    contexts: [2, 3],
                },
                UnitError::ContextsDisagree {
                    item: 1,
                    kind: None,
                    contexts: [2, 3],
                },
            ]
        }

        #[test]
        fn errors_are_tagged_by_kind() {
            for error in every_error() {
                assert!(unique(&error).starts_with("{\"kind\":"), "{error:?}");
            }
            assert_eq!(
                unique(&UnitError::Missing {
                    context: 1,
                    kind: UnitKind::SolidAngle,
                }),
                r#"{"kind":"missing","context":1,"unit_kind":"solid_angle"}"#
            );
        }

        #[test]
        fn context_units_serialize_each_kind_as_ok_or_error() {
            let g = context("#6,#12", "#16", "");
            let units = ContextUnits::of_context(&g, 100).unwrap();
            let json = unique(&units);
            assert!(
                json.contains(
                    r#""length":{"ok":{"instance":6,"kind":"length","name":"inch","factor":25.4}}"#
                ),
                "{json}"
            );
            assert!(
                json.contains(r#""solid_angle":{"error":{"kind":"missing""#),
                "{json}"
            );
            unique(&units.uncertainty().unwrap()[0]);
        }
    }
}
