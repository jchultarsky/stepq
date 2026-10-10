//! Policy constants: the limits and margins stepq applies beyond what the
//! schemas state, kept in one place.
//!
//! A policy is a choice, not a fact about STEP, so each is documented with
//! the evidence it rests on; `docs/ARCHITECTURE.md`, "Reading stated
//! geometry", lists them in one table. None of them refuses anything in the
//! 55 fixtures. Readers added later (geometry values, placements, PMI
//! routes, shapes of a definition) add their limits here.

/// Conversion-based units defined through more conversion-based units than
/// this are refused ([`UnitError::TooDeep`](crate::units::UnitError::TooDeep)),
/// as is a cycle. The deepest chain in the fixtures is two: an inch defined
/// through the centimetre, itself an SI unit (NIST FTC-07).
pub(crate) const MAX_CONVERSION_DEPTH: usize = 8;

/// A stated `ref_direction` whose sine with its placement's axis is below
/// this is refused as parallel
/// ([`GeometryErrorKind::Parallel`](crate::geometry::GeometryErrorKind::Parallel)).
/// ISO 10303-42 refuses only an exact parallel (its WR4), but projecting a
/// nearly parallel direction off the axis loses as many digits as the sine
/// has leading zeros. The smallest sine in the fixtures is 0.99999999999:
/// their axes and reference directions are all nearly orthogonal.
pub(crate) const MIN_AXIS_SINE: f64 = 1e-6;

/// A cone's semi-angle, in radians once converted, must be at least this
/// and at most π/2 minus this, or it is refused
/// ([`GeometryErrorKind::OutOfRange`](crate::geometry::GeometryErrorKind::OutOfRange)).
/// A cone is only a cone strictly between 0 and π/2, and degree factors are
/// stated with as few as 10 digits: 90 × 0.0174532925 is π/2 − 1.79e-9,
/// which an exact bound would read as a valid cone. Open CASCADE clamps a
/// semi-angle at or below 1e-12 to 1e-12 and drops a cone at or above π/2.
pub(crate) const SEMI_ANGLE_MARGIN: f64 = 1e-6;

/// Assembly trees deeper than this are not descended further, so crafted
/// input cannot exhaust the stack. The deepest fixture assembly has 7
/// levels (Project Olympus 1U).
pub(crate) const MAX_ASSEMBLY_DEPTH: usize = 1024;
