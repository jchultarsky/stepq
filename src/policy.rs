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

/// Assembly trees deeper than this are not descended further, so crafted
/// input cannot exhaust the stack. The deepest fixture assembly has 7
/// levels (Project Olympus 1U).
pub(crate) const MAX_ASSEMBLY_DEPTH: usize = 1024;
