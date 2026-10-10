//! Geometry the file states (ISO 10303-42), read by `#id`, in millimetres
//! and radians.
//!
//! A geometric item does not name its units: they are the units of the
//! representation context its representation names, and they differ
//! between representations of one file (inch parts in millimetre
//! assemblies, degree and radian contexts side by side). So the only way to
//! read a value is a [`Reader`], tied to a representation
//! ([`Reader::for_representation`]) or to a solid, shell or face whose
//! holding representations must agree ([`Reader::for_item`]). It converts
//! every length to millimetres and every plane angle to radians with the
//! factors that context states ([`ContextUnits`]); there is no
//! unscaled path and no reader built from bare factors. The raw text stays
//! available through [`p21::Record`](crate::p21::Record).
//!
//! What is read, as stated:
//!
//! * `cartesian_point` (3 coordinates, in mm), `direction` (normalised;
//!   ratios carry no unit and are never scaled), `vector` (its magnitude is
//!   a length) and `axis1_placement`;
//! * `axis2_placement_3d`, through the schema's `build_axes`
//!   ([`Axis2Placement3d`]);
//! * `plane` and the cylindrical, conical, spherical and toroidal surfaces
//!   ([`Surface`]);
//! * `line`, `circle` with a 3D placement, and `surface_curve`,
//!   `seam_curve` and `intersection_curve`, whose `curve_3d` is read one
//!   level deep and whose `master_representation` is reported ([`Curve`]).
//!
//! Any other entity, and every complex instance, is
//! [`Surface::Other`] or [`Curve::Other`], naming its type: nothing is read
//! from it and nothing is guessed. A supported entity that cannot be read
//! as stated is a [`GeometryError`] naming the innermost instance, its
//! type, the attribute and why. Values are never clamped or healed.
//!
//! ```
//! use stepq::geometry::{Reader, Surface};
//! use stepq::model::Graph;
//! use stepq::p21::parse;
//!
//! let src = b"ISO-10303-21;HEADER;FILE_SCHEMA(('AP214'));ENDSEC;DATA;
//! #1=(LENGTH_UNIT()NAMED_UNIT(*)SI_UNIT(.MILLI.,.METRE.));
//! #2=LENGTH_MEASURE_WITH_UNIT(LENGTH_MEASURE(25.4),#1);
//! #3=DIMENSIONAL_EXPONENTS(1.,0.,0.,0.,0.,0.,0.);
//! #4=(CONVERSION_BASED_UNIT('INCH',#2)LENGTH_UNIT()NAMED_UNIT(#3));
//! #5=(NAMED_UNIT(*)PLANE_ANGLE_UNIT()SI_UNIT($,.RADIAN.));
//! #6=(GEOMETRIC_REPRESENTATION_CONTEXT(3)GLOBAL_UNIT_ASSIGNED_CONTEXT((#4,#5))
//!   REPRESENTATION_CONTEXT('',''));
//! #7=SHAPE_REPRESENTATION('',(#11),#6);
//! #8=CARTESIAN_POINT('',(1.,0.,0.));
//! #9=DIRECTION('',(0.,0.,2.));
//! #10=AXIS2_PLACEMENT_3D('',#8,#9,$);
//! #11=CYLINDRICAL_SURFACE('',#10,0.25);
//! ENDSEC;END-ISO-10303-21;";
//!
//! let graph = Graph::new(parse(src)?)?;
//! let reader = Reader::for_representation(&graph, 7)?;
//! let Surface::Cylindrical(cylinder) = reader.surface(11)? else {
//!     unreachable!()
//! };
//! assert_eq!(cylinder.radius, 6.35); // 0.25 inch
//! assert_eq!(cylinder.position.location, [25.4, 0.0, 0.0]);
//! assert_eq!(cylinder.position.z, [0.0, 0.0, 1.0]); // normalised, not scaled
//! assert!(!cylinder.position.ref_direction_stated);
//! # Ok::<(), stepq::Error>(())
//! ```

use std::f64::consts::FRAC_PI_2;
use std::fmt;
use std::ops::Mul;

use crate::model::Graph;
use crate::p21::{Exchange, Instance, Literal, Param, Record};
use crate::policy::{MIN_AXIS_SINE, SEMI_ANGLE_MARGIN};
use crate::record;
use crate::units::{ContextUnits, UnitError};

/// A rigid transform in millimetres: a rotation, then a translation.
///
/// It maps coordinates in a child frame into its parent frame:
/// `p_parent = rotation · p_child + translation`. `rotation` is stored row
/// by row, `rotation[row][col]`, the layout of Open CASCADE's
/// `gp_Trsf::Value(row, col)`. `a * b` applies `b` first, so a chain of
/// frames composes from the root down: `root_from_a * a_from_b`.
///
/// Nothing re-orthonormalises the rotation: a transform carries what the
/// frames it was built from state, rounding included.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Rigid {
    /// The rotation, `rotation[row][col]`.
    pub rotation: [[f64; 3]; 3],
    /// The translation, in millimetres.
    pub translation: [f64; 3],
}

impl Rigid {
    /// The transform that moves nothing.
    pub const IDENTITY: Self = Self {
        rotation: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        translation: [0.0, 0.0, 0.0],
    };

    /// The point `p` (mm) of the child frame, in the parent frame (mm).
    pub fn apply_point(&self, p: [f64; 3]) -> [f64; 3] {
        let rotated = self.apply_vector(p);
        [
            rotated[0] + self.translation[0],
            rotated[1] + self.translation[1],
            rotated[2] + self.translation[2],
        ]
    }

    /// The vector `v` of the child frame, in the parent frame: rotated,
    /// not translated.
    pub fn apply_vector(&self, v: [f64; 3]) -> [f64; 3] {
        self.rotation.map(|row| dot(row, v))
    }

    /// The transform that undoes this one, `(Rᵀ, −Rᵀ·t)`. Exact for a
    /// rotation that is orthonormal; nothing re-orthonormalises it.
    #[must_use]
    pub fn inverse(&self) -> Self {
        let r = &self.rotation;
        let rotation = [
            [r[0][0], r[1][0], r[2][0]],
            [r[0][1], r[1][1], r[2][1]],
            [r[0][2], r[1][2], r[2][2]],
        ];
        let moved = rotation.map(|row| dot(row, self.translation));
        Self {
            rotation,
            translation: [-moved[0], -moved[1], -moved[2]],
        }
    }

    /// True if this is exactly [`Rigid::IDENTITY`]: every value equal, with
    /// no tolerance (`-0.0` equals `0.0`).
    pub fn is_identity(&self) -> bool {
        *self == Self::IDENTITY
    }
}

impl Mul for Rigid {
    type Output = Rigid;

    /// `self * rhs` applies `rhs` first, then `self`.
    fn mul(self, rhs: Rigid) -> Rigid {
        let a = &self.rotation;
        let b = &rhs.rotation;
        let column = |col: usize| [b[0][col], b[1][col], b[2][col]];
        let rotation = [0, 1, 2].map(|row| [0, 1, 2].map(|col| dot(a[row], column(col))));
        Rigid {
            rotation,
            translation: self.apply_point(rhs.translation),
        }
    }
}

/// Reads geometry values by `#id` in the units of one representation
/// context.
///
/// Lengths come back in millimetres and plane angles in radians, converted
/// with that context's stated factors when the value is read. A zero
/// length needs no unit, so an origin reads even in a context whose length
/// unit cannot be read; a non-zero length, or a cone's angle, that needs a
/// unit the context does not provide is a [`GeometryErrorKind::Unit`]
/// error.
#[derive(Debug, Clone)]
pub struct Reader<'g, 'a> {
    graph: &'g Graph<'a>,
    units: ContextUnits,
}

impl<'g, 'a> Reader<'g, 'a> {
    /// A reader in the units of the context of representation
    /// `#representation`.
    ///
    /// # Errors
    ///
    /// As [`ContextUnits::of_representation`]: the representation or its
    /// context cannot be read. A unit of one kind that cannot be read fails
    /// only the values that need it.
    pub fn for_representation(
        graph: &'g Graph<'a>,
        representation: u64,
    ) -> Result<Self, UnitError> {
        Ok(Self {
            graph,
            units: ContextUnits::of_representation(graph, representation)?,
        })
    }

    /// A reader in the units of every representation that holds `#item` (a
    /// solid, shell or face), directly or through the shells, solids and
    /// models that contain it. They must agree, kind by kind.
    ///
    /// # Errors
    ///
    /// As [`ContextUnits::of_item`]: no representation holds the item, one
    /// that does cannot be read, something that may hold it is not
    /// followed, or the holders differ in dimension. Holders that disagree
    /// on a kind of unit fail only the values that need it.
    pub fn for_item(graph: &'g Graph<'a>, item: u64) -> Result<Self, UnitError> {
        Ok(Self {
            graph,
            units: ContextUnits::of_item(graph, item)?,
        })
    }

    /// The units values are read in.
    pub fn units(&self) -> &ContextUnits {
        &self.units
    }

    /// A `cartesian_point` with three coordinates, in millimetres.
    ///
    /// # Errors
    ///
    /// A [`GeometryError`]: not a `cartesian_point`, two coordinates
    /// ([`Dimension`](GeometryErrorKind::Dimension)), a coordinate that is
    /// not a finite number, or a non-zero coordinate in a context whose
    /// length unit cannot be read.
    pub fn point(&self, id: u64) -> Result<[f64; 3], GeometryError> {
        self.read_point(id, None)
    }

    /// A `direction` with three ratios, normalised to unit length. Ratios
    /// carry no unit and are never scaled; they are divided by the largest
    /// of them before squaring, so neither tiny nor huge ratios overflow.
    ///
    /// # Errors
    ///
    /// A [`GeometryError`]: not a `direction`, two ratios, a ratio that is
    /// not a finite number, or every ratio exactly zero
    /// ([`ZeroDirection`](GeometryErrorKind::ZeroDirection)).
    pub fn direction(&self, id: u64) -> Result<[f64; 3], GeometryError> {
        self.read_direction(id, None)
    }

    /// A `vector`: its orientation, normalised, and its magnitude in
    /// millimetres.
    ///
    /// # Errors
    ///
    /// A [`GeometryError`], from the vector or its orientation; a negative
    /// magnitude is [`OutOfRange`](GeometryErrorKind::OutOfRange).
    pub fn vector(&self, id: u64) -> Result<Vector, GeometryError> {
        self.read_vector(id, None)
    }

    /// An `axis1_placement`. Without a stated axis it is the schema's
    /// default, (0, 0, 1).
    ///
    /// # Errors
    ///
    /// A [`GeometryError`], from the placement, its location or its axis.
    pub fn axis1_placement(&self, id: u64) -> Result<Axis1Placement, GeometryError> {
        let params = self.simple(id, None, "AXIS1_PLACEMENT", 3)?;
        let location = self.read_point(
            self.required(id, &params[1], "location")?,
            Some((id, "location")),
        )?;
        let axis = self.optional_direction(id, &params[2], "axis")?;
        Ok(Axis1Placement {
            instance: id,
            location,
            axis: axis.unwrap_or(Z),
            axis_stated: axis.is_some(),
        })
    }

    /// An `axis2_placement_3d`, its axes built as the schema's
    /// `build_axes` builds them ([`Axis2Placement3d`]).
    ///
    /// # Errors
    ///
    /// A [`GeometryError`], from the placement, its location or its
    /// directions; an `axis2_placement_2d` is
    /// [`Dimension`](GeometryErrorKind::Dimension), and a `ref_direction`
    /// parallel to the axis is [`Parallel`](GeometryErrorKind::Parallel).
    pub fn axis2_placement_3d(&self, id: u64) -> Result<Axis2Placement3d, GeometryError> {
        self.read_axis2_placement_3d(id, None)
    }

    /// A surface: a `plane` or a cylindrical, conical, spherical or
    /// toroidal surface. Any other entity, and every complex instance, is
    /// [`Surface::Other`]; this does not check that it is a surface at all.
    ///
    /// # Errors
    ///
    /// A [`GeometryError`] if `#id` is not defined, or a supported surface
    /// cannot be read as stated: the wrong number of attributes, a
    /// placement that cannot be read, a radius that is not positive (a
    /// cone's may be zero), or a cone's semi-angle outside
    /// [10⁻⁶, π/2 − 10⁻⁶] rad once converted.
    pub fn surface(&self, id: u64) -> Result<Surface, GeometryError> {
        let instance = self.entry(id, None)?;
        let Some(record) = self.simple_record(instance) else {
            return Ok(Surface::Other(self.other(id, instance)));
        };
        let surface = if record.is("PLANE") {
            let params = self.params(id, record, 2)?;
            Surface::Plane(Plane {
                instance: id,
                position: self.position(id, &params)?,
            })
        } else if record.is("CYLINDRICAL_SURFACE") {
            let params = self.params(id, record, 3)?;
            Surface::Cylindrical(CylindricalSurface {
                instance: id,
                position: self.position(id, &params)?,
                radius: self.positive_length(id, &params[2], "radius")?,
            })
        } else if record.is("CONICAL_SURFACE") {
            let params = self.params(id, record, 4)?;
            let position = self.position(id, &params)?;
            let radius = self.number(id, &params[2], "radius")?;
            if radius < 0.0 {
                return Err(self.error(id, Some("radius"), out_of_range(radius)));
            }
            Surface::Conical(ConicalSurface {
                instance: id,
                position,
                radius: self.length(id, "radius", radius)?,
                semi_angle: self.semi_angle(id, &params[3])?,
            })
        } else if record.is("SPHERICAL_SURFACE") {
            let params = self.params(id, record, 3)?;
            Surface::Spherical(SphericalSurface {
                instance: id,
                position: self.position(id, &params)?,
                radius: self.positive_length(id, &params[2], "radius")?,
            })
        } else if record.is("TOROIDAL_SURFACE") {
            let params = self.params(id, record, 4)?;
            Surface::Toroidal(ToroidalSurface {
                instance: id,
                position: self.position(id, &params)?,
                major_radius: self.positive_length(id, &params[2], "major_radius")?,
                minor_radius: self.positive_length(id, &params[3], "minor_radius")?,
            })
        } else {
            Surface::Other(self.other(id, instance))
        };
        Ok(surface)
    }

    /// A curve: a `line`, a `circle` with a 3D placement, or a
    /// `surface_curve`, `seam_curve` or `intersection_curve`, whose
    /// `curve_3d` is read one level deep (a surface curve inside it is
    /// [`Curve::Other`]) and whose pcurves are not read. Any other entity,
    /// and every complex instance, is [`Curve::Other`]; this does not check
    /// that it is a curve at all.
    ///
    /// # Errors
    ///
    /// A [`GeometryError`] if `#id` is not defined, or a supported curve
    /// cannot be read as stated: the wrong number of attributes, a point,
    /// vector or placement that cannot be read, a radius that is not
    /// positive, or a `master_representation` that is not `.CURVE_3D.`,
    /// `.PCURVE_S1.` or `.PCURVE_S2.`.
    pub fn curve(&self, id: u64) -> Result<Curve, GeometryError> {
        self.read_curve(id, None, true)
    }

    fn exchange(&self) -> &'g Exchange<'a> {
        self.graph.exchange()
    }

    /// A [`GeometryError`] at `#id`, with its entity types.
    fn error(
        &self,
        id: u64,
        attribute: Option<&'static str>,
        kind: GeometryErrorKind,
    ) -> GeometryError {
        let exchange = self.exchange();
        GeometryError {
            instance: id,
            entity: exchange
                .get(id)
                .map(|instance| record::entity(exchange, instance))
                .unwrap_or_default(),
            attribute,
            kind,
        }
    }

    /// Instance `#id`, read from attribute `from.1` of `#from.0` or, if
    /// `from` is `None`, asked for by the caller. A missing instance is
    /// reported at the instance that refers to it.
    fn entry(&self, id: u64, from: Origin) -> Result<&'g Instance, GeometryError> {
        self.exchange().get(id).ok_or_else(|| match from {
            Some((holder, attribute)) => {
                self.error(holder, Some(attribute), GeometryErrorKind::Missing)
            }
            None => self.error(id, None, GeometryErrorKind::Missing),
        })
    }

    /// The record of a simple instance; `None` for a complex one.
    fn simple_record(&self, instance: &Instance) -> Option<Record<'g>> {
        if instance.is_complex() {
            return None;
        }
        self.exchange().records(instance).next()
    }

    fn other(&self, id: u64, instance: &Instance) -> OtherGeometry {
        OtherGeometry {
            instance: id,
            entity: record::entity(self.exchange(), instance),
        }
    }

    /// The parameters of simple instance `#id`, which must be a `name`
    /// with `count` attributes.
    fn simple(
        &self,
        id: u64,
        from: Origin,
        name: &'static str,
        count: usize,
    ) -> Result<Vec<Param<'g>>, GeometryError> {
        let instance = self.entry(id, from)?;
        if instance.is_complex() {
            return Err(self.error(id, None, GeometryErrorKind::Complex));
        }
        match self.exchange().records(instance).next() {
            Some(record) if record.is(name) => self.params(id, record, count),
            _ => Err(self.error(id, None, GeometryErrorKind::WrongType { expected: name })),
        }
    }

    /// The parameters of `record`, which must number `count`.
    fn params(
        &self,
        id: u64,
        record: Record<'g>,
        count: usize,
    ) -> Result<Vec<Param<'g>>, GeometryError> {
        record::exact_params(record, count).ok_or_else(|| {
            self.error(
                id,
                None,
                GeometryErrorKind::ParameterCount {
                    expected: count,
                    found: record.params().count(),
                },
            )
        })
    }

    /// A reference attribute; `None` if unset.
    fn reference(
        &self,
        id: u64,
        param: &Param<'_>,
        attribute: &'static str,
    ) -> Result<Option<u64>, GeometryError> {
        match param {
            Param::Reference(target) => Ok(Some(*target)),
            Param::Unset => Ok(None),
            _ => Err(self.error(id, Some(attribute), GeometryErrorKind::NotAReference)),
        }
    }

    /// A reference attribute the schema requires.
    fn required(
        &self,
        id: u64,
        param: &Param<'_>,
        attribute: &'static str,
    ) -> Result<u64, GeometryError> {
        self.reference(id, param, attribute)?
            .ok_or_else(|| self.error(id, Some(attribute), GeometryErrorKind::MissingAttribute))
    }

    /// A REAL or INTEGER attribute, finite.
    fn number(
        &self,
        id: u64,
        param: &Param<'_>,
        attribute: &'static str,
    ) -> Result<f64, GeometryError> {
        if let Some(value) = record::real(param) {
            return Ok(value);
        }
        let kind = match param {
            Param::Integer(literal) | Param::Real(literal) if literal.to_f64().is_some() => {
                GeometryErrorKind::NotFinite
            }
            Param::Unset => GeometryErrorKind::MissingAttribute,
            _ => GeometryErrorKind::NotANumber,
        };
        Err(self.error(id, Some(attribute), kind))
    }

    /// The three numbers of a `coordinates` or `direction_ratios` list.
    fn triple(
        &self,
        id: u64,
        param: &Param<'_>,
        attribute: &'static str,
    ) -> Result<[f64; 3], GeometryError> {
        let Some(items) = param.list() else {
            let kind = if param.is_unset() {
                GeometryErrorKind::MissingAttribute
            } else {
                GeometryErrorKind::NotAList
            };
            return Err(self.error(id, Some(attribute), kind));
        };
        let items: Vec<Param<'_>> = items.collect();
        let [a, b, c] = items.as_slice() else {
            return Err(self.error(
                id,
                Some(attribute),
                GeometryErrorKind::Dimension { found: items.len() },
            ));
        };
        Ok([
            self.number(id, a, attribute)?,
            self.number(id, b, attribute)?,
            self.number(id, c, attribute)?,
        ])
    }

    /// `value`, a length in the context's unit, in millimetres. Zero needs
    /// no unit.
    fn length(&self, id: u64, attribute: &'static str, value: f64) -> Result<f64, GeometryError> {
        if value == 0.0 {
            return Ok(value);
        }
        let unit = self
            .units
            .length()
            .map_err(|e| self.error(id, Some(attribute), GeometryErrorKind::Unit(e)))?;
        let millimetres = value * unit.factor;
        if !millimetres.is_finite() {
            return Err(self.error(id, Some(attribute), GeometryErrorKind::NotFinite));
        }
        Ok(millimetres)
    }

    /// A `positive_length_measure` attribute, in millimetres.
    fn positive_length(
        &self,
        id: u64,
        param: &Param<'_>,
        attribute: &'static str,
    ) -> Result<f64, GeometryError> {
        let value = self.number(id, param, attribute)?;
        if value <= 0.0 {
            return Err(self.error(id, Some(attribute), out_of_range(value)));
        }
        self.length(id, attribute, value)
    }

    /// A conical surface's `semi_angle`, in radians: no closer than
    /// `SEMI_ANGLE_MARGIN` to 0 or π/2 once converted.
    fn semi_angle(&self, id: u64, param: &Param<'_>) -> Result<f64, GeometryError> {
        const ATTRIBUTE: &str = "semi_angle";
        let value = self.number(id, param, ATTRIBUTE)?;
        let refuse = || self.error(id, Some(ATTRIBUTE), out_of_range(value));
        if value <= 0.0 {
            return Err(refuse());
        }
        let unit = self
            .units
            .plane_angle()
            .map_err(|e| self.error(id, Some(ATTRIBUTE), GeometryErrorKind::Unit(e)))?;
        let radians = value * unit.factor;
        if !(SEMI_ANGLE_MARGIN..=FRAC_PI_2 - SEMI_ANGLE_MARGIN).contains(&radians) {
            return Err(refuse());
        }
        Ok(radians)
    }

    fn read_point(&self, id: u64, from: Origin) -> Result<[f64; 3], GeometryError> {
        const ATTRIBUTE: &str = "coordinates";
        let params = self.simple(id, from, "CARTESIAN_POINT", 2)?;
        let [x, y, z] = self.triple(id, &params[1], ATTRIBUTE)?;
        Ok([
            self.length(id, ATTRIBUTE, x)?,
            self.length(id, ATTRIBUTE, y)?,
            self.length(id, ATTRIBUTE, z)?,
        ])
    }

    fn read_direction(&self, id: u64, from: Origin) -> Result<[f64; 3], GeometryError> {
        const ATTRIBUTE: &str = "direction_ratios";
        let params = self.simple(id, from, "DIRECTION", 2)?;
        let ratios = self.triple(id, &params[1], ATTRIBUTE)?;
        normalise(ratios)
            .ok_or_else(|| self.error(id, Some(ATTRIBUTE), GeometryErrorKind::ZeroDirection))
    }

    /// An optional direction attribute of `#id`.
    fn optional_direction(
        &self,
        id: u64,
        param: &Param<'_>,
        attribute: &'static str,
    ) -> Result<Option<[f64; 3]>, GeometryError> {
        self.reference(id, param, attribute)?
            .map(|direction| self.read_direction(direction, Some((id, attribute))))
            .transpose()
    }

    fn read_vector(&self, id: u64, from: Origin) -> Result<Vector, GeometryError> {
        let params = self.simple(id, from, "VECTOR", 3)?;
        let orientation = self.read_direction(
            self.required(id, &params[1], "orientation")?,
            Some((id, "orientation")),
        )?;
        let magnitude = self.number(id, &params[2], "magnitude")?;
        if magnitude < 0.0 {
            return Err(self.error(id, Some("magnitude"), out_of_range(magnitude)));
        }
        Ok(Vector {
            instance: id,
            orientation,
            magnitude: self.length(id, "magnitude", magnitude)?,
        })
    }

    fn read_axis2_placement_3d(
        &self,
        id: u64,
        from: Origin,
    ) -> Result<Axis2Placement3d, GeometryError> {
        let instance = self.entry(id, from)?;
        if self
            .simple_record(instance)
            .is_some_and(|record| record.is("AXIS2_PLACEMENT_2D"))
        {
            return Err(self.error(id, None, GeometryErrorKind::Dimension { found: 2 }));
        }
        let params = self.simple(id, from, "AXIS2_PLACEMENT_3D", 4)?;
        let location = self.read_point(
            self.required(id, &params[1], "location")?,
            Some((id, "location")),
        )?;
        let axis = self.optional_direction(id, &params[2], "axis")?;
        let ref_direction = self.optional_direction(id, &params[3], "ref_direction")?;
        let z = axis.unwrap_or(Z);
        let [x, y] = build_axes(z, ref_direction)
            .ok_or_else(|| self.error(id, Some("ref_direction"), GeometryErrorKind::Parallel))?;
        Ok(Axis2Placement3d {
            instance: id,
            location,
            x,
            y,
            z,
            axis_stated: axis.is_some(),
            ref_direction_stated: ref_direction.is_some(),
        })
    }

    /// The `position` of an elementary surface, its second attribute.
    fn position(&self, id: u64, params: &[Param<'_>]) -> Result<Axis2Placement3d, GeometryError> {
        let position = self.required(id, &params[1], "position")?;
        self.read_axis2_placement_3d(position, Some((id, "position")))
    }

    /// A curve; surface curves are read only if `unwrap` (not inside
    /// another one).
    fn read_curve(&self, id: u64, from: Origin, unwrap: bool) -> Result<Curve, GeometryError> {
        let instance = self.entry(id, from)?;
        let Some(record) = self.simple_record(instance) else {
            return Ok(Curve::Other(self.other(id, instance)));
        };
        let curve = if record.is("LINE") {
            let params = self.params(id, record, 3)?;
            let pnt = self.read_point(self.required(id, &params[1], "pnt")?, Some((id, "pnt")))?;
            let dir = self.read_vector(self.required(id, &params[2], "dir")?, Some((id, "dir")))?;
            Curve::Line(Line {
                instance: id,
                pnt,
                dir,
            })
        } else if record.is("CIRCLE") {
            let params = self.params(id, record, 3)?;
            Curve::Circle(Circle {
                instance: id,
                position: self.position(id, &params)?,
                radius: self.positive_length(id, &params[2], "radius")?,
            })
        } else if let (true, Some(subtype)) = (unwrap, SurfaceCurveKind::of(record)) {
            let params = self.params(id, record, 4)?;
            let curve_3d = self.required(id, &params[1], "curve_3d")?;
            let curve_3d = self.read_curve(curve_3d, Some((id, "curve_3d")), false)?;
            if params[2].list().is_none() {
                return Err(self.error(
                    id,
                    Some("associated_geometry"),
                    GeometryErrorKind::NotAList,
                ));
            }
            Curve::SurfaceCurve(SurfaceCurve {
                instance: id,
                subtype,
                curve_3d: Box::new(curve_3d),
                master_representation: self.master_representation(id, &params[3])?,
            })
        } else {
            Curve::Other(self.other(id, instance))
        };
        Ok(curve)
    }

    fn master_representation(
        &self,
        id: u64,
        param: &Param<'_>,
    ) -> Result<MasterRepresentation, GeometryError> {
        const ATTRIBUTE: &str = "master_representation";
        if param.is_unset() {
            return Err(self.error(id, Some(ATTRIBUTE), GeometryErrorKind::MissingAttribute));
        }
        let value = param.literal().and_then(Literal::enumeration);
        [
            ("CURVE_3D", MasterRepresentation::Curve3d),
            ("PCURVE_S1", MasterRepresentation::PcurveS1),
            ("PCURVE_S2", MasterRepresentation::PcurveS2),
        ]
        .into_iter()
        .find(|(name, _)| value.is_some_and(|value| value.eq_ignore_ascii_case(name)))
        .map(|(_, master)| master)
        .ok_or_else(|| self.error(id, Some(ATTRIBUTE), GeometryErrorKind::UnknownEnumeration))
    }
}

/// Where a reference was read from: the instance holding it and the
/// attribute; `None` for an `#id` the caller asked for.
type Origin = Option<(u64, &'static str)>;

/// The schema's default axis.
const Z: [f64; 3] = [0.0, 0.0, 1.0];

fn out_of_range(value: f64) -> GeometryErrorKind {
    GeometryErrorKind::OutOfRange { value }
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

/// `v` scaled to unit length; `None` if every component is zero. Dividing
/// by the largest magnitude first keeps the squares in range for any
/// finite input.
fn normalise(v: [f64; 3]) -> Option<[f64; 3]> {
    let largest = v.iter().fold(0.0f64, |m, c| m.max(c.abs()));
    if largest == 0.0 {
        return None;
    }
    let scaled = v.map(|c| c / largest);
    let length = dot(scaled, scaled).sqrt();
    Some(scaled.map(|c| c / length))
}

/// The x and y axes of ISO 10303-42's `build_axes` for unit axis `z`, as
/// AP203 edition 2, AP214 edition 3 and AP242 edition 4 define its
/// `first_proj_axis`: a stated `ref_direction` projected off `z`, or by
/// default (1, 0, 0) projected off `z`, or (0, 1, 0) when `z` is exactly
/// ±(1, 0, 0). `None` if a stated `ref_direction` is parallel to `z`, to
/// within [`MIN_AXIS_SINE`].
fn build_axes(z: [f64; 3], ref_direction: Option<[f64; 3]>) -> Option<[[f64; 3]; 2]> {
    let projected = if let Some(v) = ref_direction {
        let normal = cross(v, z);
        if dot(normal, normal).sqrt() < MIN_AXIS_SINE {
            return None;
        }
        let along = dot(v, z);
        [
            v[0] - along * z[0],
            v[1] - along * z[1],
            v[2] - along * z[2],
        ]
    } else {
        #[expect(
            clippy::float_cmp,
            reason = "the schema's first_proj_axis compares the normalised axis exactly"
        )]
        let along_x = z == [1.0, 0.0, 0.0] || z == [-1.0, 0.0, 0.0];
        // The default is a unit vector e_k, projected off z: e_k − z_k·z.
        // Its k-th component, 1 − z_k², is the sum of the other two squares
        // for a unit z; written so, it keeps its digits when z is nearly
        // ±e_k, where 1 − z_k² cancels and leaves x out of square with z.
        let k = usize::from(along_x);
        let mut projected = [0.0; 3];
        for (i, component) in projected.iter_mut().enumerate() {
            *component = if i == k {
                (0..3).filter(|&j| j != k).map(|j| z[j] * z[j]).sum()
            } else {
                0.0 - z[k] * z[i]
            };
        }
        projected
    };
    let x = normalise(projected)?;
    let y = normalise(cross(z, x))?;
    Some([x, y])
}

/// A `vector`: a direction and a length.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct Vector {
    /// The `#id` of the `vector`.
    pub instance: u64,
    /// Its `orientation`, normalised.
    pub orientation: [f64; 3],
    /// Its `magnitude`, a length, in millimetres; never negative.
    pub magnitude: f64,
}

/// An `axis1_placement`: a point and an axis.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct Axis1Placement {
    /// The `#id` of the `axis1_placement`.
    pub instance: u64,
    /// Its `location`, in millimetres.
    pub location: [f64; 3],
    /// Its `axis`, normalised; (0, 0, 1) if none is stated.
    pub axis: [f64; 3],
    /// True if the file states the axis.
    pub axis_stated: bool,
}

/// An `axis2_placement_3d`: a location and three orthonormal axes.
///
/// The axes are ISO 10303-42's `build_axes`, as AP203 edition 2, AP214
/// edition 3 and AP242 edition 4 define its `first_proj_axis`:
///
/// * `z` is the stated `axis`, normalised, or (0, 0, 1);
/// * `x` is the stated `ref_direction` projected off `z` and normalised,
///   or, if none is stated, (1, 0, 0) projected off `z`, or (0, 1, 0) when
///   `z` is exactly ±(1, 0, 0);
/// * `y` is `z × x`, normalised.
///
/// A stated `ref_direction` within 10⁻⁶ (as a sine) of the axis is refused
/// as [`Parallel`](GeometryErrorKind::Parallel): the schema refuses only
/// exact parallels, and below that margin the projection loses most of
/// its digits.
///
/// Where `ref_direction` is not stated, `x` is the schema's default, which
/// only sets a surface's parameterisation; Open CASCADE builds a different
/// default for some axes, so a caller to whom `x` matters checks
/// `ref_direction_stated`. AP203 edition 1 (`CONFIG_CONTROL_DESIGN`) leaves
/// `x` undefined when `z` is exactly (−1, 0, 0); stepq reads it with the
/// later editions' default, (0, 1, 0), and `ref_direction_stated` is false.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct Axis2Placement3d {
    /// The `#id` of the `axis2_placement_3d`.
    pub instance: u64,
    /// Its `location`, in millimetres.
    pub location: [f64; 3],
    /// The x axis.
    pub x: [f64; 3],
    /// The y axis.
    pub y: [f64; 3],
    /// The z axis.
    pub z: [f64; 3],
    /// True if the file states `axis`; otherwise `z` is (0, 0, 1).
    pub axis_stated: bool,
    /// True if the file states `ref_direction`; otherwise `x` is the
    /// schema's default.
    pub ref_direction_stated: bool,
}

impl Axis2Placement3d {
    /// The transform from the frame this placement defines to the frame it
    /// is stated in, in millimetres: the columns of its rotation are `x`,
    /// `y` and `z`, and its translation is `location`.
    pub fn to_rigid(&self) -> Rigid {
        let (x, y, z) = (self.x, self.y, self.z);
        Rigid {
            rotation: [[x[0], y[0], z[0]], [x[1], y[1], z[1]], [x[2], y[2], z[2]]],
            translation: self.location,
        }
    }
}

/// A surface, as [`Reader::surface`] reads it.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "serde", serde(tag = "kind", rename_all = "snake_case"))]
#[non_exhaustive]
pub enum Surface {
    /// A `plane`.
    Plane(Plane),
    /// A `cylindrical_surface`.
    Cylindrical(CylindricalSurface),
    /// A `conical_surface`.
    Conical(ConicalSurface),
    /// A `spherical_surface`.
    Spherical(SphericalSurface),
    /// A `toroidal_surface`.
    Toroidal(ToroidalSurface),
    /// Not read: a B-spline surface, a surface of revolution or of linear
    /// extrusion, an offset or trimmed surface, a
    /// `degenerate_toroidal_surface`, any other entity and every complex
    /// instance. The caller treats it as unknown.
    Other(OtherGeometry),
}

impl Surface {
    /// The `#id` of the surface.
    pub fn instance(&self) -> u64 {
        match self {
            Self::Plane(s) => s.instance,
            Self::Cylindrical(s) => s.instance,
            Self::Conical(s) => s.instance,
            Self::Spherical(s) => s.instance,
            Self::Toroidal(s) => s.instance,
            Self::Other(s) => s.instance,
        }
    }
}

/// A `plane`: the plane through `position.location` with normal
/// `position.z`.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct Plane {
    /// The `#id` of the `plane`.
    pub instance: u64,
    /// Its `position`.
    pub position: Axis2Placement3d,
}

/// A `cylindrical_surface`: its axis is the line through
/// `position.location` along `position.z`.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct CylindricalSurface {
    /// The `#id` of the `cylindrical_surface`.
    pub instance: u64,
    /// Its `position`.
    pub position: Axis2Placement3d,
    /// Its `radius`, in millimetres; positive.
    pub radius: f64,
}

/// A `conical_surface`. It has radius `radius` in the plane of
/// `position.location` and widens along `position.z`; its apex is not
/// computed.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct ConicalSurface {
    /// The `#id` of the `conical_surface`.
    pub instance: u64,
    /// Its `position`.
    pub position: Axis2Placement3d,
    /// Its `radius`, in millimetres; zero or positive.
    pub radius: f64,
    /// Its `semi_angle`, in radians, within [10⁻⁶, π/2 − 10⁻⁶].
    pub semi_angle: f64,
}

/// A `spherical_surface`, centred on `position.location`.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct SphericalSurface {
    /// The `#id` of the `spherical_surface`.
    pub instance: u64,
    /// Its `position`.
    pub position: Axis2Placement3d,
    /// Its `radius`, in millimetres; positive.
    pub radius: f64,
}

/// A `toroidal_surface`, centred on `position.location` with its axis
/// along `position.z`.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct ToroidalSurface {
    /// The `#id` of the `toroidal_surface`.
    pub instance: u64,
    /// Its `position`.
    pub position: Axis2Placement3d,
    /// Its `major_radius`, in millimetres; positive.
    pub major_radius: f64,
    /// Its `minor_radius`, in millimetres; positive.
    pub minor_radius: f64,
}

/// A curve, as [`Reader::curve`] reads it.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "serde", serde(tag = "kind", rename_all = "snake_case"))]
#[non_exhaustive]
pub enum Curve {
    /// A `line`.
    Line(Line),
    /// A `circle` with a 3D placement.
    Circle(Circle),
    /// A `surface_curve`, `seam_curve` or `intersection_curve`.
    SurfaceCurve(SurfaceCurve),
    /// Not read: a B-spline curve, an ellipse, a trimmed, composite or
    /// polyline curve, a pcurve, any other entity, every complex instance,
    /// and a surface curve inside a surface curve. The caller treats it as
    /// unknown.
    Other(OtherGeometry),
}

impl Curve {
    /// The `#id` of the curve.
    pub fn instance(&self) -> u64 {
        match self {
            Self::Line(c) => c.instance,
            Self::Circle(c) => c.instance,
            Self::SurfaceCurve(c) => c.instance,
            Self::Other(c) => c.instance,
        }
    }

    /// The 3D curve: a surface curve's `curve_3d`, otherwise the curve
    /// itself.
    pub fn basis(&self) -> &Curve {
        match self {
            Self::SurfaceCurve(c) => &c.curve_3d,
            _ => self,
        }
    }
}

/// A `line`: through `pnt`, along `dir`.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct Line {
    /// The `#id` of the `line`.
    pub instance: u64,
    /// Its `pnt`, in millimetres.
    pub pnt: [f64; 3],
    /// Its `dir`. The magnitude sets the line's parameterisation.
    pub dir: Vector,
}

/// A `circle` in the plane of `position.x` and `position.y`, centred on
/// `position.location`.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct Circle {
    /// The `#id` of the `circle`.
    pub instance: u64,
    /// Its `position`.
    pub position: Axis2Placement3d,
    /// Its `radius`, in millimetres; positive.
    pub radius: f64,
}

/// A `surface_curve`, `seam_curve` or `intersection_curve`: a 3D curve that
/// also lies on one or two surfaces. Only `curve_3d` is read.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct SurfaceCurve {
    /// The `#id` of the surface curve.
    pub instance: u64,
    /// Which entity it is.
    pub subtype: SurfaceCurveKind,
    /// Its `curve_3d`, read one level deep: a surface curve inside it is
    /// [`Curve::Other`].
    pub curve_3d: Box<Curve>,
    /// Which representation the file says is the master. With
    /// `pcurve_s1` or `pcurve_s2` the schema prefers the curve on the
    /// surface, which stepq does not read, so `curve_3d` may be the
    /// approximation; whether to trust it is the caller's decision.
    pub master_representation: MasterRepresentation,
}

/// Which surface curve entity a [`SurfaceCurve`] is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
#[non_exhaustive]
pub enum SurfaceCurveKind {
    /// `surface_curve`.
    SurfaceCurve,
    /// `seam_curve`: the curve along which a closed surface meets itself.
    SeamCurve,
    /// `intersection_curve`: where two surfaces meet.
    IntersectionCurve,
}

impl SurfaceCurveKind {
    /// The kind a simple record is, if it is one of the three.
    fn of(record: Record<'_>) -> Option<Self> {
        [
            ("SURFACE_CURVE", Self::SurfaceCurve),
            ("SEAM_CURVE", Self::SeamCurve),
            ("INTERSECTION_CURVE", Self::IntersectionCurve),
        ]
        .into_iter()
        .find(|(name, _)| record.is(name))
        .map(|(_, kind)| kind)
    }
}

/// A surface curve's `master_representation`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum MasterRepresentation {
    /// `.CURVE_3D.`: the 3D curve is the master.
    #[cfg_attr(feature = "serde", serde(rename = "curve_3d"))]
    Curve3d,
    /// `.PCURVE_S1.`: the curve on the first associated surface is.
    #[cfg_attr(feature = "serde", serde(rename = "pcurve_s1"))]
    PcurveS1,
    /// `.PCURVE_S2.`: the curve on the second associated surface is.
    #[cfg_attr(feature = "serde", serde(rename = "pcurve_s2"))]
    PcurveS2,
}

/// An entity that is not read: a type stepq does not interpret, or a
/// complex instance.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct OtherGeometry {
    /// Its `#id`.
    pub instance: u64,
    /// Its entity type, upper case; for a complex instance, its partial
    /// entities joined with `+` in the order written.
    pub entity: String,
}

/// Why a geometry value cannot be read as stated.
///
/// `instance` is the innermost instance at fault: a zero `direction` inside
/// a cylinder's placement is reported at the `direction`, with attribute
/// `direction_ratios`. A reference to an `#id` that is not defined is
/// reported at the instance that holds the reference, with that attribute;
/// an `#id` asked for that is not defined, at that `#id`, with no entity.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct GeometryError {
    /// The instance at fault.
    pub instance: u64,
    /// Its upper-cased entity types, joined with `+`; empty if it is not
    /// defined.
    pub entity: String,
    /// The attribute at fault, by its schema name, if one is.
    pub attribute: Option<&'static str>,
    /// What is wrong.
    pub kind: GeometryErrorKind,
}

impl fmt::Display for GeometryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{}", self.instance)?;
        if !self.entity.is_empty() {
            write!(f, " ({})", self.entity)?;
        }
        if let Some(attribute) = self.attribute {
            write!(f, " {attribute}")?;
        }
        write!(f, ": {}", self.kind)
    }
}

/// What is wrong with a geometry value; see [`GeometryError`].
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "serde", serde(tag = "kind", rename_all = "snake_case"))]
#[non_exhaustive]
pub enum GeometryErrorKind {
    /// No instance has the `#id`.
    Missing,
    /// Not the entity a point, direction, vector or placement must be.
    WrongType {
        /// The entity expected, upper case.
        expected: &'static str,
    },
    /// A complex instance where a point, direction, vector or placement
    /// must be.
    Complex,
    /// A record with another number of attributes than the schema gives
    /// its entity: it does not have the layout read here.
    ParameterCount {
        /// The schema's count.
        expected: usize,
        /// The record's.
        found: usize,
    },
    /// `$` where the schema requires a value.
    MissingAttribute,
    /// Not a REAL or INTEGER token: a typed value such as
    /// `LENGTH_MEASURE(5.)`, a string, `*`, a list.
    NotANumber,
    /// Not a reference.
    NotAReference,
    /// Not a list.
    NotAList,
    /// A number too large for an `f64`, as written or once converted.
    NotFinite,
    /// A point, direction or placement that is not 3D.
    Dimension {
        /// Its number of coordinates or ratios.
        found: usize,
    },
    /// A direction whose every ratio is exactly zero (its WR1).
    ZeroDirection,
    /// A stated `ref_direction` parallel to the axis, to within a sine of
    /// 10⁻⁶.
    Parallel,
    /// A value outside its range, as written: a radius not positive (a
    /// cone's negative), a negative vector magnitude, or a cone's
    /// semi-angle outside [10⁻⁶, π/2 − 10⁻⁶] rad once converted.
    OutOfRange {
        /// The value as written, before conversion.
        value: f64,
    },
    /// A `master_representation` that is not `.CURVE_3D.`, `.PCURVE_S1.`
    /// or `.PCURVE_S2.`.
    UnknownEnumeration,
    /// A non-zero length, or a cone's semi-angle, whose unit the context
    /// does not provide.
    #[cfg_attr(feature = "serde", serde(serialize_with = "unit_error"))]
    Unit(UnitError),
}

/// Serializes [`GeometryErrorKind::Unit`]'s error under `error`: its own
/// `kind` tag would otherwise meet the variant's.
#[cfg(feature = "serde")]
fn unit_error<S: serde::Serializer>(error: &UnitError, serializer: S) -> Result<S::Ok, S::Error> {
    use serde::ser::SerializeMap;
    let mut map = serializer.serialize_map(Some(1))?;
    map.serialize_entry("error", error)?;
    map.end()
}

impl fmt::Display for GeometryErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing => f.write_str("not defined"),
            Self::WrongType { expected } => write!(f, "not a {expected}"),
            Self::Complex => f.write_str("a complex instance, which is not read as one entity"),
            Self::ParameterCount { expected, found } => write!(
                f,
                "{found} attributes where the schema has {expected}, so it is not read"
            ),
            Self::MissingAttribute => f.write_str("unset, but the schema requires a value"),
            Self::NotANumber => f.write_str("not a number"),
            Self::NotAReference => f.write_str("not a reference"),
            Self::NotAList => f.write_str("not a list"),
            Self::NotFinite => f.write_str("not a finite number"),
            Self::Dimension { found } => {
                write!(f, "{found} coordinates; only 3D geometry is read")
            }
            Self::ZeroDirection => f.write_str("every direction ratio is zero"),
            Self::Parallel => f.write_str("parallel to the axis (a sine below 1e-6)"),
            Self::OutOfRange { value } => write!(f, "{value} is out of range"),
            Self::UnknownEnumeration => f.write_str("not .CURVE_3D., .PCURVE_S1. or .PCURVE_S2."),
            Self::Unit(error) => write!(f, "its unit cannot be read: {error}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::p21::parse;
    use crate::units::UnitKind;

    /// Units: the millimetre, the radian, an inch over the millimetre, and a
    /// degree stated with 10 and with 13 digits.
    const UNITS: &str = "#1=(LENGTH_UNIT()NAMED_UNIT(*)SI_UNIT(.MILLI.,.METRE.));\
        #2=(NAMED_UNIT(*)PLANE_ANGLE_UNIT()SI_UNIT($,.RADIAN.));\
        #3=DIMENSIONAL_EXPONENTS(1.,0.,0.,0.,0.,0.,0.);\
        #4=LENGTH_MEASURE_WITH_UNIT(LENGTH_MEASURE(25.4),#1);\
        #5=(CONVERSION_BASED_UNIT('INCH',#4)LENGTH_UNIT()NAMED_UNIT(#3));\
        #6=DIMENSIONAL_EXPONENTS(0.,0.,0.,0.,0.,0.,0.);\
        #7=PLANE_ANGLE_MEASURE_WITH_UNIT(PLANE_ANGLE_MEASURE(0.0174532925),#2);\
        #8=(CONVERSION_BASED_UNIT('DEGREE',#7)NAMED_UNIT(#6)PLANE_ANGLE_UNIT());\
        #9=PLANE_ANGLE_MEASURE_WITH_UNIT(PLANE_ANGLE_MEASURE(0.01745329251994),#2);\
        #10=(CONVERSION_BASED_UNIT('DEGREE',#9)NAMED_UNIT(#6)PLANE_ANGLE_UNIT());\
        #20=(GEOMETRIC_REPRESENTATION_CONTEXT(3)GLOBAL_UNIT_ASSIGNED_CONTEXT((#1,#2))\
            REPRESENTATION_CONTEXT('',''));\
        #21=(GEOMETRIC_REPRESENTATION_CONTEXT(3)GLOBAL_UNIT_ASSIGNED_CONTEXT((#5,#8))\
            REPRESENTATION_CONTEXT('',''));\
        #22=(GEOMETRIC_REPRESENTATION_CONTEXT(3)GLOBAL_UNIT_ASSIGNED_CONTEXT((#1,#10))\
            REPRESENTATION_CONTEXT('',''));\
        #23=(GEOMETRIC_REPRESENTATION_CONTEXT(3)GLOBAL_UNIT_ASSIGNED_CONTEXT((#1))\
            REPRESENTATION_CONTEXT('',''));\
        #24=(GEOMETRIC_REPRESENTATION_CONTEXT(3)GLOBAL_UNIT_ASSIGNED_CONTEXT((#2))\
            REPRESENTATION_CONTEXT('',''));\
        #30=SHAPE_REPRESENTATION('',(),#20);\
        #31=SHAPE_REPRESENTATION('',(),#21);\
        #32=SHAPE_REPRESENTATION('',(),#22);\
        #33=SHAPE_REPRESENTATION('',(),#23);\
        #34=SHAPE_REPRESENTATION('',(),#24);";

    /// Millimetres and radians.
    const MM: u64 = 30;
    /// Inches and a degree of 0.0174532925 rad.
    const INCH: u64 = 31;
    /// Millimetres and a degree of 0.01745329251994 rad.
    const MM_DEGREE: u64 = 32;
    /// Millimetres and no plane angle unit.
    const NO_ANGLE: u64 = 33;
    /// Radians and no length unit.
    const NO_LENGTH: u64 = 34;

    /// Instances shared by the tests: the origin, the three axes, and the
    /// placement they make.
    const FRAME: &str = "#100=CARTESIAN_POINT('',(0.,0.,0.));\
        #101=DIRECTION('',(1.,0.,0.));#102=DIRECTION('',(0.,1.,0.));\
        #103=DIRECTION('',(0.,0.,1.));#104=AXIS2_PLACEMENT_3D('',#100,#103,#101);";

    fn graph_in(schema: &str, data: &str) -> &'static Graph<'static> {
        let src = format!(
            "ISO-10303-21;HEADER;FILE_SCHEMA(('{schema}'));ENDSEC;DATA;\
             {UNITS}{FRAME}{data}ENDSEC;END-ISO-10303-21;"
        );
        let src: &'static [u8] = Box::leak(src.into_bytes().into_boxed_slice());
        Box::leak(Box::new(Graph::build(parse(src).unwrap())))
    }

    /// A reader in the units of `representation`, over `data`.
    fn reader(representation: u64, data: &str) -> Reader<'static, 'static> {
        Reader::for_representation(
            graph_in("AP242_MANAGED_MODEL_BASED_3D_ENGINEERING_MIM_LF", data),
            representation,
        )
        .unwrap()
    }

    fn close(a: [f64; 3], b: [f64; 3]) -> bool {
        a.iter().zip(b).all(|(a, b)| (a - b).abs() <= 1e-15)
    }

    fn bits(v: [f64; 3]) -> [u64; 3] {
        v.map(f64::to_bits)
    }

    fn kind<T: fmt::Debug>(result: Result<T, GeometryError>) -> GeometryErrorKind {
        result.unwrap_err().kind
    }

    fn surface(representation: u64, data: &str, id: u64) -> Surface {
        reader(representation, data).surface(id).unwrap()
    }

    #[test]
    fn points_are_read_in_millimetres() {
        let data = "#200=CARTESIAN_POINT('',(1.,-2.5,0.5));#201=CARTESIAN_POINT('',(1,2,3));";
        assert_eq!(reader(MM, data).point(200), Ok([1.0, -2.5, 0.5]));
        assert_eq!(
            bits(reader(INCH, data).point(200).unwrap()),
            bits([25.4, -2.5 * 25.4, 0.5 * 25.4])
        );
        assert_eq!(
            reader(MM, data).point(201),
            Ok([1.0, 2.0, 3.0]),
            "INTEGER tokens are read as numbers"
        );
    }

    #[test]
    fn directions_are_normalised_and_never_scaled() {
        let data = "#200=DIRECTION('',(0.,0.,5.));#201=DIRECTION('',(0.,0.,0.));\
            #202=DIRECTION('',(1.,0.));#203=DIRECTION('',(1.E999,0.,0.));\
            #204=DIRECTION('',(1.E-200,0.,0.));#205=DIRECTION('',(-1.E200,0.,0.));\
            #206=DIRECTION('',(3.E-162,4.E-162,0.));#207=DIRECTION('',(1.E155,1.E155,0.));\
            #208=DIRECTION('',(0.,-0.,-0.03937));";
        let mm = reader(MM, data);
        let inch = reader(INCH, data);
        assert_eq!(mm.direction(200), Ok([0.0, 0.0, 1.0]));
        assert_eq!(
            inch.direction(200),
            Ok([0.0, 0.0, 1.0]),
            "ratios have no unit"
        );
        let zero = mm.direction(201).unwrap_err();
        assert_eq!(
            (zero.instance, zero.attribute, zero.kind),
            (
                201,
                Some("direction_ratios"),
                GeometryErrorKind::ZeroDirection
            )
        );
        assert_eq!(
            kind(mm.direction(202)),
            GeometryErrorKind::Dimension { found: 2 }
        );
        assert_eq!(kind(mm.direction(203)), GeometryErrorKind::NotFinite);
        // Extreme but legal ratios: neither squaring underflows nor
        // overflows, and the result is unit length.
        assert_eq!(mm.direction(204), Ok([1.0, 0.0, 0.0]));
        assert_eq!(mm.direction(205), Ok([-1.0, 0.0, 0.0]));
        let tiny = mm.direction(206).unwrap();
        assert!(close(tiny, [0.6, 0.8, 0.0]), "{tiny:?}");
        assert!((dot(tiny, tiny) - 1.0).abs() <= 1e-15);
        let huge = mm.direction(207).unwrap();
        assert!(close(huge, [0.5f64.sqrt(), 0.5f64.sqrt(), 0.0]), "{huge:?}");
        assert_eq!(
            mm.direction(208),
            Ok([0.0, -0.0, -1.0]),
            "a ratio scaled as if it were inches is still a direction"
        );
    }

    #[test]
    fn vectors_have_a_length_and_a_direction() {
        let data = "#200=VECTOR('',#103,2.);#201=DIRECTION('',(0.,3.,4.));#202=VECTOR('',#201,1);\
            #203=VECTOR('',#103,-1.);#204=VECTOR('',#103,0.);";
        let inch = reader(INCH, data);
        let vector = inch.vector(200).unwrap();
        assert_eq!(
            (vector.instance, vector.orientation, vector.magnitude),
            (200, [0.0, 0.0, 1.0], 2.0 * 25.4)
        );
        let tilted = inch.vector(202).unwrap();
        assert!(close(tilted.orientation, [0.0, 0.6, 0.8]));
        assert_eq!(tilted.magnitude, 25.4);
        let negative = inch.vector(203).unwrap_err();
        assert_eq!(
            (negative.attribute, negative.kind),
            (
                Some("magnitude"),
                GeometryErrorKind::OutOfRange { value: -1.0 }
            )
        );
        assert_eq!(
            reader(NO_LENGTH, data).vector(204).map(|v| v.magnitude),
            Ok(0.0),
            "a zero length needs no unit"
        );
    }

    #[test]
    fn axis1_placements_default_their_axis() {
        let data = "#200=CARTESIAN_POINT('',(1.,2.,3.));#201=AXIS1_PLACEMENT('',#200,$);\
            #202=DIRECTION('',(0.,-2.,0.));#203=AXIS1_PLACEMENT('',#200,#202);";
        let inch = reader(INCH, data);
        let unset = inch.axis1_placement(201).unwrap();
        assert_eq!(
            (unset.location, unset.axis, unset.axis_stated),
            ([25.4, 2.0 * 25.4, 3.0 * 25.4], [0.0, 0.0, 1.0], false)
        );
        let stated = inch.axis1_placement(203).unwrap();
        assert_eq!((stated.axis, stated.axis_stated), ([0.0, -1.0, 0.0], true));
    }

    /// A placement at the origin with `axis` and `ref_direction` as given
    /// (`$` for unset), as instance #200, with its directions #201, #202.
    fn placement(
        representation: u64,
        axis: &str,
        ref_direction: &str,
    ) -> Result<Axis2Placement3d, GeometryError> {
        let direction = |id, ratios: &str| {
            if ratios == "$" {
                ("$".to_owned(), String::new())
            } else {
                (format!("#{id}"), format!("#{id}=DIRECTION('',({ratios}));"))
            }
        };
        let (axis, axis_line) = direction(201, axis);
        let (ref_direction, ref_line) = direction(202, ref_direction);
        reader(
            representation,
            &format!(
                "{axis_line}{ref_line}#200=AXIS2_PLACEMENT_3D('',#100,{axis},{ref_direction});"
            ),
        )
        .axis2_placement_3d(200)
    }

    #[test]
    fn placements_without_directions_are_the_identity_frame() {
        let frame = placement(MM, "$", "$").unwrap();
        assert_eq!(
            (frame.x, frame.y, frame.z),
            ([1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0])
        );
        assert!(!frame.axis_stated && !frame.ref_direction_stated);
        assert!(frame.to_rigid().is_identity());
        // At the origin, no length unit is needed.
        assert!(
            placement(NO_LENGTH, "$", "$")
                .unwrap()
                .to_rigid()
                .is_identity()
        );
    }

    /// ISO 10303-42's `first_proj_axis`: (1, 0, 0) projected off z, or
    /// (0, 1, 0) when z is exactly ±(1, 0, 0). Open CASCADE 8.0.1 builds
    /// x = (0, 0, 1) for the axes (1, 0, 0) and (0, 1, 0) instead.
    #[test]
    fn a_defaulted_ref_direction_follows_the_schema() {
        for (axis, x) in [
            ("0.,0.,1.", [1.0, 0.0, 0.0]),
            ("1.,0.,0.", [0.0, 1.0, 0.0]),
            ("-1.,0.,0.", [0.0, 1.0, 0.0]),
            ("0.,1.,0.", [1.0, 0.0, 0.0]),
            ("0.,0.,-1.", [1.0, 0.0, 0.0]),
            ("2.,0.,0.", [0.0, 1.0, 0.0]),
        ] {
            let frame = placement(MM, axis, "$").unwrap();
            assert_eq!(frame.x, x, "axis {axis}");
            assert!(frame.axis_stated && !frame.ref_direction_stated);
            assert_eq!(cross(frame.x, frame.y), frame.z, "axis {axis}");
        }
        let tilted = placement(MM, "0.6,0.,0.8", "$").unwrap();
        assert!(close(tilted.x, [0.8, 0.0, -0.6]), "{:?}", tilted.x);
        assert!(close(tilted.y, [0.0, 1.0, 0.0]), "{:?}", tilted.y);
        // Just off x, the default is ill-conditioned but defined, and the
        // frame stays square: (1, 0, 0) projected off z keeps its digits.
        for axis in [
            "1.,2.E-8,0.",
            "-1.,0.,3.E-9",
            "1.,1.E-300,0.",
            "1.,1.E-12,1.E-12",
        ] {
            let frame = placement(MM, axis, "$").unwrap();
            for (a, b) in [(frame.x, frame.z), (frame.y, frame.z), (frame.x, frame.y)] {
                assert!(dot(a, b).abs() <= 1e-15, "axis {axis}: {frame:?}");
            }
            assert!(close(cross(frame.x, frame.y), frame.z), "axis {axis}");
        }
        let near = placement(MM, "1.,2.E-8,0.", "$").unwrap();
        assert!(close(near.x, [2e-8, -1.0, 0.0]), "{:?}", near.x);
    }

    /// AP203 edition 1 (`CONFIG_CONTROL_DESIGN`) tests only z ≠ (1, 0, 0), so
    /// for z = (−1, 0, 0) without a `ref_direction` its own schema leaves x
    /// undefined. stepq reads the later editions' default and says that x
    /// was not stated; gear, clevis21 and valve have six such PLANE and
    /// CIRCLE positions, where x only sets the parameterisation.
    #[test]
    fn ap203_edition_1_minus_x_reads_the_later_default() {
        let graph = graph_in(
            "CONFIG_CONTROL_DESIGN",
            "#200=DIRECTION('',(-1.,0.,0.));#201=AXIS2_PLACEMENT_3D('',#100,#200,$);\
             #202=PLANE('',#201);",
        );
        let reader = Reader::for_representation(graph, MM).unwrap();
        let Surface::Plane(plane) = reader.surface(202).unwrap() else {
            panic!("a plane");
        };
        let frame = plane.position;
        assert_eq!(
            (frame.x, frame.y, frame.z),
            ([0.0, 1.0, 0.0], [0.0, 0.0, -1.0], [-1.0, 0.0, 0.0])
        );
        assert!(frame.axis_stated);
        assert!(!frame.ref_direction_stated, "x is the schema's default");
    }

    #[test]
    fn a_stated_ref_direction_is_projected_off_the_axis() {
        // |cos| = 3.3e-6, as in the NIST AP242 plane positions.
        let frame = placement(MM, "0.,0.,1.", "1.,0.,3.3E-6").unwrap();
        assert_eq!(frame.x, [1.0, 0.0, 0.0]);
        assert_eq!(frame.y, [0.0, 1.0, 0.0]);
        assert!(frame.ref_direction_stated);
        for (axis, reference) in [
            ("0.,0.,1.", "1.,0.,0."),
            ("0.,1.,0.", "0.,0.,1."),
            ("1.,0.,0.", "0.,-1.,0."),
            ("0.,0.,-1.", "0.,1.,0."),
        ] {
            let frame = placement(MM, axis, reference).unwrap();
            assert_eq!(
                cross(frame.x, frame.y),
                frame.z,
                "x × y = z exactly for axis {axis}, ref {reference}"
            );
        }
        let skew = placement(MM, "1.,1.,1.", "1.,0.,0.").unwrap();
        for (a, b) in [(skew.x, skew.y), (skew.y, skew.z), (skew.z, skew.x)] {
            assert!(dot(a, b).abs() <= 1e-15);
        }
        assert!(close(cross(skew.x, skew.y), skew.z));
    }

    #[test]
    fn a_ref_direction_parallel_to_the_axis_is_refused() {
        for reference in ["0.,0.,2.", "0.,0.,-1.", "5.E-7,0.,1."] {
            let error = placement(MM, "0.,0.,1.", reference).unwrap_err();
            assert_eq!(
                (error.instance, error.attribute, error.kind),
                (200, Some("ref_direction"), GeometryErrorKind::Parallel),
                "{reference}"
            );
        }
        assert_eq!(
            placement(MM, "0.,0.,1.", "2.E-6,0.,1.").map(|frame| frame.x),
            Ok([1.0, 0.0, 0.0]),
            "a sine above 1e-6 is read"
        );
    }

    #[test]
    fn each_surface_is_read_in_the_context_units() {
        let data = "#200=PLANE('',#104);#201=CYLINDRICAL_SURFACE('',#104,0.25);\
            #202=CONICAL_SURFACE('',#104,0.25,0.5);#203=SPHERICAL_SURFACE('',#104,0.25);\
            #204=TOROIDAL_SURFACE('',#104,2.,0.25);#205=CONICAL_SURFACE('',#104,0.25,59.);\
            #206=CONICAL_SURFACE('',#104,0.,1.);";
        let frame = reader(MM, data).axis2_placement_3d(104).unwrap();
        assert_eq!(
            surface(MM, data, 200),
            Surface::Plane(Plane {
                instance: 200,
                position: frame.clone()
            })
        );
        let Surface::Cylindrical(cylinder) = surface(INCH, data, 201) else {
            panic!("a cylinder");
        };
        assert_eq!((cylinder.instance, cylinder.radius), (201, 6.35));
        assert_eq!(cylinder.position, frame);
        let Surface::Conical(cone) = surface(MM, data, 202) else {
            panic!("a cone");
        };
        assert_eq!((cone.radius, cone.semi_angle), (0.25, 0.5));
        let Surface::Spherical(sphere) = surface(INCH, data, 203) else {
            panic!("a sphere");
        };
        assert_eq!(sphere.radius, 6.35);
        let Surface::Toroidal(torus) = surface(INCH, data, 204) else {
            panic!("a torus");
        };
        assert_eq!((torus.major_radius, torus.minor_radius), (2.0 * 25.4, 6.35));
        // A drill point: 59 degrees, with the degree as the file states it.
        let Surface::Conical(drill) = surface(INCH, data, 205) else {
            panic!("a cone");
        };
        assert_eq!(
            drill.semi_angle.to_bits(),
            (59.0 * 0.017_453_292_5f64).to_bits()
        );
        assert_eq!(drill.radius, 6.35);
        // The trap: 1 in a degree context is a degree, not a radian.
        let Surface::Conical(small) = surface(INCH, data, 206) else {
            panic!("a cone");
        };
        assert_eq!(small.semi_angle.to_bits(), 0.017_453_292_5f64.to_bits());
        assert_eq!(small.radius, 0.0, "a cone's radius may be zero");
    }

    #[test]
    fn values_out_of_range_are_refused_as_written() {
        let data = "#200=CYLINDRICAL_SURFACE('',#104,0.);#201=CYLINDRICAL_SURFACE('',#104,-1.);\
            #202=TOROIDAL_SURFACE('',#104,2.,0.);#203=CONICAL_SURFACE('',#104,-1.,0.5);\
            #204=CONICAL_SURFACE('',#104,1.,0.);#205=CONICAL_SURFACE('',#104,1.,90.);\
            #206=CONICAL_SURFACE('',#104,1.,1.5709);#207=CONICAL_SURFACE('',#104,1.,-10.);\
            #208=SPHERICAL_SURFACE('',#104,-2.);#209=CIRCLE('',#104,0.);\
            #210=CONICAL_SURFACE('',#104,1.,89.9999);";
        let refused = |representation, id, attribute, value: f64| {
            let error = reader(representation, data).surface(id).unwrap_err();
            assert_eq!(
                (error.instance, error.attribute, error.kind),
                (id, Some(attribute), GeometryErrorKind::OutOfRange { value }),
                "#{id}"
            );
        };
        refused(MM, 200, "radius", 0.0);
        refused(MM, 201, "radius", -1.0);
        refused(MM, 202, "minor_radius", 0.0);
        refused(MM, 203, "radius", -1.0);
        refused(MM, 204, "semi_angle", 0.0);
        refused(MM, 206, "semi_angle", 1.5709);
        refused(MM, 207, "semi_angle", -10.0);
        refused(MM, 208, "radius", -2.0);
        // 90 degrees with truncated degree factors is just below π/2 (by
        // 1.8e-9 and 4.0e-13) and still refused.
        refused(INCH, 205, "semi_angle", 90.0);
        refused(MM_DEGREE, 205, "semi_angle", 90.0);
        assert!(
            matches!(surface(MM_DEGREE, data, 210), Surface::Conical(_)),
            "89.9999 degrees is π/2 − 1.7e-6"
        );
        assert_eq!(
            kind(reader(MM, data).curve(209)),
            GeometryErrorKind::OutOfRange { value: 0.0 }
        );
    }

    #[test]
    fn a_unit_is_needed_only_by_what_uses_it() {
        let data = "#200=CARTESIAN_POINT('',(1.,0.,0.));#201=CYLINDRICAL_SURFACE('',#104,1.);\
            #202=CONICAL_SURFACE('',#104,1.,0.5);#203=CONICAL_SURFACE('',#104,1.,0.);";
        let no_length = reader(NO_LENGTH, data);
        assert_eq!(
            no_length.point(100),
            Ok([0.0; 3]),
            "the origin needs no unit"
        );
        let error = no_length.point(200).unwrap_err();
        assert_eq!(
            (error.instance, error.attribute, error.kind),
            (
                200,
                Some("coordinates"),
                GeometryErrorKind::Unit(UnitError::Missing {
                    context: 24,
                    kind: UnitKind::Length
                })
            )
        );
        assert!(matches!(
            kind(no_length.surface(201)),
            GeometryErrorKind::Unit(UnitError::Missing { .. })
        ));
        let no_angle = reader(NO_ANGLE, data);
        assert!(matches!(no_angle.surface(201), Ok(Surface::Cylindrical(_))));
        let cone = no_angle.surface(202).unwrap_err();
        assert_eq!(
            (cone.attribute, cone.kind),
            (
                Some("semi_angle"),
                GeometryErrorKind::Unit(UnitError::Missing {
                    context: 23,
                    kind: UnitKind::PlaneAngle
                })
            )
        );
        assert_eq!(
            kind(no_angle.surface(203)),
            GeometryErrorKind::OutOfRange { value: 0.0 },
            "a zero angle is out of range in any unit"
        );
    }

    #[test]
    fn unread_entities_and_complex_instances_are_other() {
        let data = "#200=B_SPLINE_SURFACE_WITH_KNOTS('',1,1,((#100,#100),(#100,#100)),.UNSPECIFIED.,\
            .F.,.F.,.F.,(2),(2),(0.,1.),(0.,1.),.UNSPECIFIED.);\
            #201=SURFACE_OF_REVOLUTION('',#300,#302);#202=ELLIPSE('',#104,2.,1.);\
            #203=TRIMMED_CURVE('',#300,(#100),(#100),.T.,.CARTESIAN.);\
            #204=(BOUNDED_SURFACE()B_SPLINE_SURFACE(1,1,((#100,#100),(#100,#100)),.UNSPECIFIED.,.F.,.F.,.F.)\
            B_SPLINE_SURFACE_WITH_KNOTS((2),(2),(0.,1.),(0.,1.),.UNSPECIFIED.)GEOMETRIC_REPRESENTATION_ITEM()\
            RATIONAL_B_SPLINE_SURFACE(((1.,1.),(1.,1.)))REPRESENTATION_ITEM('')SURFACE());\
            #205=(CYLINDRICAL_SURFACE('',#104,1.)GEOMETRIC_REPRESENTATION_ITEM());\
            #206=DEGENERATE_TOROIDAL_SURFACE('',#104,1.,2.,.T.);\
            #300=LINE('',#100,#301);#301=VECTOR('',#101,1.);#302=AXIS1_PLACEMENT('',#100,#103);";
        let mm = reader(MM, data);
        let other = |entity: &str, id| OtherGeometry {
            instance: id,
            entity: entity.to_owned(),
        };
        assert_eq!(
            mm.surface(200),
            Ok(Surface::Other(other("B_SPLINE_SURFACE_WITH_KNOTS", 200)))
        );
        assert_eq!(
            mm.surface(201),
            Ok(Surface::Other(other("SURFACE_OF_REVOLUTION", 201)))
        );
        assert_eq!(mm.curve(202), Ok(Curve::Other(other("ELLIPSE", 202))));
        assert_eq!(mm.curve(203), Ok(Curve::Other(other("TRIMMED_CURVE", 203))));
        assert_eq!(
            mm.surface(204),
            Ok(Surface::Other(other(
                "BOUNDED_SURFACE+B_SPLINE_SURFACE+B_SPLINE_SURFACE_WITH_KNOTS+\
                 GEOMETRIC_REPRESENTATION_ITEM+RATIONAL_B_SPLINE_SURFACE+REPRESENTATION_ITEM+SURFACE",
                204
            )))
        );
        assert_eq!(
            mm.surface(205),
            Ok(Surface::Other(other(
                "CYLINDRICAL_SURFACE+GEOMETRIC_REPRESENTATION_ITEM",
                205
            ))),
            "a complex instance is not read, even around an elementary record"
        );
        assert_eq!(
            mm.surface(206),
            Ok(Surface::Other(other("DEGENERATE_TOROIDAL_SURFACE", 206)))
        );
        assert_eq!(mm.surface(205).unwrap().instance(), 205);
    }

    #[test]
    fn lines_and_circles() {
        let data = "#200=CARTESIAN_POINT('',(1.,0.,0.));#201=VECTOR('',#102,3.);\
            #202=LINE('',#200,#201);#203=CIRCLE('',#104,0.5);\
            #204=AXIS2_PLACEMENT_2D('',#205,$);#205=CARTESIAN_POINT('',(0.,0.));\
            #206=CIRCLE('',#204,1.);";
        let inch = reader(INCH, data);
        let Curve::Line(line) = inch.curve(202).unwrap() else {
            panic!("a line");
        };
        assert_eq!(line.pnt, [25.4, 0.0, 0.0]);
        assert_eq!(
            (line.dir.instance, line.dir.orientation, line.dir.magnitude),
            (201, [0.0, 1.0, 0.0], 3.0 * 25.4)
        );
        let Curve::Circle(circle) = inch.curve(203).unwrap() else {
            panic!("a circle");
        };
        assert_eq!((circle.instance, circle.radius), (203, 0.5 * 25.4));
        assert_eq!(circle.position.instance, 104);
        let flat = inch.curve(206).unwrap_err();
        assert_eq!(
            (flat.instance, flat.entity.as_str(), flat.kind),
            (
                204,
                "AXIS2_PLACEMENT_2D",
                GeometryErrorKind::Dimension { found: 2 }
            ),
            "a circle in a 2D placement is not read"
        );
    }

    #[test]
    fn surface_curves_are_unwrapped_one_level() {
        let data = "#200=LINE('',#100,#201);#201=VECTOR('',#101,1.);#202=CIRCLE('',#104,2.);\
            #203=B_SPLINE_CURVE_WITH_KNOTS('',1,(#100,#100),.UNSPECIFIED.,.F.,.F.,(2),(0.,1.),.UNSPECIFIED.);\
            #210=SURFACE_CURVE('',#200,(#300),.PCURVE_S2.);\
            #211=SEAM_CURVE('',#202,(#300,#300),.PCURVE_S1.);\
            #212=INTERSECTION_CURVE('',#203,(#300,#300),.CURVE_3D.);\
            #213=SURFACE_CURVE('',#211,(#300),.CURVE_3D.);\
            #214=SURFACE_CURVE('',#200,(#300),.FOO.);\
            #215=surface_curve('',#202,(#300),.curve_3d.);\
            #216=SURFACE_CURVE('',#200,(#300),$);\
            #217=SURFACE_CURVE('',#200,#300,.CURVE_3D.);\
            #300=PLANE('',#104);";
        let mm = reader(MM, data);
        let unwrap = |id| match mm.curve(id).unwrap() {
            Curve::SurfaceCurve(curve) => curve,
            other => panic!("#{id}: {other:?}"),
        };
        let surface = unwrap(210);
        assert_eq!(
            (surface.subtype, surface.master_representation),
            (
                SurfaceCurveKind::SurfaceCurve,
                MasterRepresentation::PcurveS2
            )
        );
        assert!(matches!(*surface.curve_3d, Curve::Line(_)));
        let seam = unwrap(211);
        assert_eq!(
            (seam.subtype, seam.master_representation),
            (SurfaceCurveKind::SeamCurve, MasterRepresentation::PcurveS1)
        );
        assert!(matches!(*seam.curve_3d, Curve::Circle(_)));
        let intersection = unwrap(212);
        assert_eq!(
            (intersection.subtype, intersection.master_representation),
            (
                SurfaceCurveKind::IntersectionCurve,
                MasterRepresentation::Curve3d
            )
        );
        assert_eq!(
            *intersection.curve_3d,
            Curve::Other(OtherGeometry {
                instance: 203,
                entity: "B_SPLINE_CURVE_WITH_KNOTS".to_owned()
            })
        );
        assert_eq!(
            *unwrap(213).curve_3d,
            Curve::Other(OtherGeometry {
                instance: 211,
                entity: "SEAM_CURVE".to_owned()
            }),
            "a surface curve inside a surface curve is not read"
        );
        let bad = mm.curve(214).unwrap_err();
        assert_eq!(
            (bad.instance, bad.attribute, bad.kind),
            (
                214,
                Some("master_representation"),
                GeometryErrorKind::UnknownEnumeration
            )
        );
        assert_eq!(
            unwrap(215).master_representation,
            MasterRepresentation::Curve3d,
            "names and enumerations in lower case"
        );
        assert_eq!(kind(mm.curve(216)), GeometryErrorKind::MissingAttribute);
        assert_eq!(kind(mm.curve(217)), GeometryErrorKind::NotAList);
        let seam = mm.curve(211).unwrap();
        assert_eq!(seam.basis().instance(), 202);
        let line = mm.curve(200).unwrap();
        assert_eq!(line.basis(), &line);
    }

    #[test]
    fn errors_name_the_innermost_instance() {
        let data = "#200=DIRECTION('',(0.,0.,0.));#201=AXIS2_PLACEMENT_3D('',#100,#200,$);\
            #202=CYLINDRICAL_SURFACE('',#201,1.);#203=CYLINDRICAL_SURFACE('',#998,1.);\
            #204=AXIS2_PLACEMENT_3D('',#100,#103,#999);#205=PLANE('',#204);";
        let mm = reader(MM, data);
        let error = mm.surface(202).unwrap_err();
        assert_eq!(
            error,
            GeometryError {
                instance: 200,
                entity: "DIRECTION".to_owned(),
                attribute: Some("direction_ratios"),
                kind: GeometryErrorKind::ZeroDirection,
            }
        );
        // A dangling reference is reported where it is held.
        assert_eq!(
            mm.surface(203).unwrap_err(),
            GeometryError {
                instance: 203,
                entity: "CYLINDRICAL_SURFACE".to_owned(),
                attribute: Some("position"),
                kind: GeometryErrorKind::Missing,
            }
        );
        let deep = mm.surface(205).unwrap_err();
        assert_eq!(
            (deep.instance, deep.attribute, deep.kind),
            (204, Some("ref_direction"), GeometryErrorKind::Missing)
        );
        assert_eq!(
            mm.surface(997).unwrap_err(),
            GeometryError {
                instance: 997,
                entity: String::new(),
                attribute: None,
                kind: GeometryErrorKind::Missing,
            }
        );
    }

    #[test]
    fn malformed_records_are_refused() {
        let data = "#200=CYLINDRICAL_SURFACE('',#104,$);\
            #201=CYLINDRICAL_SURFACE('',#104,LENGTH_MEASURE(5.));\
            #202=CYLINDRICAL_SURFACE('',#104,'5');#203=CYLINDRICAL_SURFACE('',#104);\
            #204=CYLINDRICAL_SURFACE('',#104,5.,7.);#205=CYLINDRICAL_SURFACE('',#104,5);\
            #206=cylindrical_surface('',#104,5.);#207=CYLINDRICAL_SURFACE('','x',5.);\
            #208=CARTESIAN_POINT('',(1.,2.));#209=(CARTESIAN_POINT('',(1.,2.,3.))\
            GEOMETRIC_REPRESENTATION_ITEM());#210=PLANE('',#104,#104);\
            #211=CARTESIAN_POINT('',$);#212=CARTESIAN_POINT('',(1.,'2',3.));\
            #213=AXIS2_PLACEMENT_3D('',$,$,$);#214=CYLINDRICAL_SURFACE('',#104,1.E999);";
        let mm = reader(MM, data);
        let refusal = |id| {
            let error = mm.surface(id).unwrap_err();
            (error.instance, error.attribute, error.kind)
        };
        assert_eq!(
            refusal(200),
            (200, Some("radius"), GeometryErrorKind::MissingAttribute)
        );
        assert_eq!(
            refusal(201),
            (201, Some("radius"), GeometryErrorKind::NotANumber)
        );
        assert_eq!(
            refusal(202),
            (202, Some("radius"), GeometryErrorKind::NotANumber)
        );
        assert_eq!(
            refusal(203),
            (
                203,
                None,
                GeometryErrorKind::ParameterCount {
                    expected: 3,
                    found: 2
                }
            )
        );
        assert_eq!(
            refusal(204),
            (
                204,
                None,
                GeometryErrorKind::ParameterCount {
                    expected: 3,
                    found: 4
                }
            ),
            "one parameter too many"
        );
        assert!(
            matches!(mm.surface(205), Ok(Surface::Cylindrical(c)) if c.radius.to_bits() == 5.0f64.to_bits())
        );
        assert!(matches!(mm.surface(206), Ok(Surface::Cylindrical(_))));
        assert_eq!(
            refusal(207),
            (207, Some("position"), GeometryErrorKind::NotAReference)
        );
        assert_eq!(
            refusal(210),
            (
                210,
                None,
                GeometryErrorKind::ParameterCount {
                    expected: 2,
                    found: 3
                }
            )
        );
        assert_eq!(
            refusal(214),
            (214, Some("radius"), GeometryErrorKind::NotFinite)
        );
        assert_eq!(
            kind(mm.point(208)),
            GeometryErrorKind::Dimension { found: 2 }
        );
        assert_eq!(kind(mm.point(209)), GeometryErrorKind::Complex);
        assert_eq!(kind(mm.point(211)), GeometryErrorKind::MissingAttribute);
        assert_eq!(kind(mm.point(212)), GeometryErrorKind::NotANumber);
        assert_eq!(
            kind(mm.point(103)),
            GeometryErrorKind::WrongType {
                expected: "CARTESIAN_POINT"
            }
        );
        assert_eq!(
            kind(mm.axis2_placement_3d(213)),
            GeometryErrorKind::MissingAttribute,
            "location is required"
        );
        assert_eq!(
            kind(mm.vector(103)),
            GeometryErrorKind::WrongType { expected: "VECTOR" }
        );
    }

    #[test]
    fn a_reader_for_an_item_takes_the_units_of_its_holders() {
        let data = "#200=CYLINDRICAL_SURFACE('',#104,0.5);#201=ADVANCED_FACE('',(),#200,.T.);\
            #202=CLOSED_SHELL('',(#201));#203=MANIFOLD_SOLID_BREP('',#202);\
            #204=ADVANCED_BREP_SHAPE_REPRESENTATION('',(#203),#21);";
        let graph = graph_in("AP214", data);
        let reader = Reader::for_item(graph, 201).unwrap();
        assert_eq!(reader.units().length().unwrap().factor, 25.4);
        assert!(
            matches!(reader.surface(200), Ok(Surface::Cylindrical(c)) if c.radius.to_bits() == (0.5 * 25.4f64).to_bits())
        );
        assert_eq!(
            Reader::for_item(graph, 200).unwrap_err(),
            UnitError::NotHeld { item: 200 }
        );
        assert!(Reader::for_representation(graph, 203).is_err());
    }

    #[test]
    fn rigid_transforms_compose_child_to_parent() {
        // A quarter turn about z, then a shift along x.
        let turn = Rigid {
            rotation: [[0.0, -1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]],
            translation: [10.0, 0.0, 0.0],
        };
        let shift = Rigid {
            translation: [0.0, 0.0, 5.0],
            ..Rigid::IDENTITY
        };
        assert!(Rigid::IDENTITY.is_identity());
        assert!(!turn.is_identity());
        assert!(
            Rigid {
                translation: [-0.0, 0.0, 0.0],
                ..Rigid::IDENTITY
            }
            .is_identity()
        );
        assert_eq!(turn.apply_point([1.0, 0.0, 0.0]), [10.0, 1.0, 0.0]);
        assert_eq!(turn.apply_vector([1.0, 0.0, 0.0]), [0.0, 1.0, 0.0]);
        // `a * b` applies b first.
        let point = [1.0, 2.0, 3.0];
        assert_eq!(
            (turn * shift).apply_point(point),
            turn.apply_point(shift.apply_point(point))
        );
        assert_eq!((turn * shift).translation, [10.0, 0.0, 5.0]);
        assert_eq!((shift * turn).translation, [10.0, 0.0, 5.0]);
        assert_ne!((turn * turn).translation, (turn * shift).translation);
        assert!((turn.inverse() * turn).is_identity());
        assert!((turn * turn.inverse()).is_identity());
        assert_eq!(
            turn.inverse().apply_point([10.0, 1.0, 0.0]),
            [1.0, 0.0, 0.0]
        );
        assert_eq!(Rigid::IDENTITY * turn, turn);
    }

    #[test]
    fn a_placement_maps_its_frame_into_the_enclosing_one() {
        let data = "#200=CARTESIAN_POINT('',(1.,2.,3.));\
            #201=AXIS2_PLACEMENT_3D('',#200,#101,#102);";
        let frame = reader(INCH, data).axis2_placement_3d(201).unwrap();
        assert_eq!(
            (frame.x, frame.y, frame.z),
            ([0.0, 1.0, 0.0], [0.0, 0.0, 1.0], [1.0, 0.0, 0.0])
        );
        let rigid = frame.to_rigid();
        assert_eq!(rigid.apply_vector([1.0, 0.0, 0.0]), frame.x);
        assert_eq!(rigid.apply_vector([0.0, 0.0, 1.0]), frame.z);
        assert_eq!(rigid.apply_point([0.0, 0.0, 0.0]), frame.location);
        assert_eq!(frame.location, [25.4, 2.0 * 25.4, 3.0 * 25.4]);
    }

    #[test]
    fn errors_format_and_wrap() {
        let mm = reader(MM, "#200=CYLINDRICAL_SURFACE('',#104,-1.);");
        let error = mm.surface(200).unwrap_err();
        assert_eq!(
            error.to_string(),
            "#200 (CYLINDRICAL_SURFACE) radius: -1 is out of range"
        );
        assert_eq!(
            crate::Error::from(error.clone()).to_string(),
            error.to_string()
        );
        assert_eq!(
            mm.surface(999).unwrap_err().to_string(),
            "#999: not defined"
        );
        let unit = reader(NO_LENGTH, "#200=CARTESIAN_POINT('',(1.,0.,0.));")
            .point(200)
            .unwrap_err();
        assert_eq!(
            unit.to_string(),
            "#200 (CARTESIAN_POINT) coordinates: its unit cannot be read: \
             context #24 assigns no length unit"
        );
    }

    #[cfg(feature = "serde")]
    mod serialized {
        use super::*;
        use crate::testing::unique;

        #[test]
        fn every_result_type_has_unique_keys() {
            let data = "#200=PLANE('',#104);#201=CYLINDRICAL_SURFACE('',#104,1.);\
                #202=CONICAL_SURFACE('',#104,1.,0.5);#203=SPHERICAL_SURFACE('',#104,1.);\
                #204=TOROIDAL_SURFACE('',#104,2.,1.);#205=ELLIPSE('',#104,2.,1.);\
                #206=LINE('',#100,#207);#207=VECTOR('',#101,1.);#208=CIRCLE('',#104,1.);\
                #209=SEAM_CURVE('',#208,(#200,#200),.PCURVE_S1.);\
                #210=AXIS1_PLACEMENT('',#100,$);";
            let mm = reader(MM, data);
            for id in 200..=205 {
                let json = unique(&mm.surface(id).unwrap());
                assert!(json.starts_with("{\"kind\":"), "{json}");
            }
            for id in [205, 206, 208, 209] {
                let json = unique(&mm.curve(id).unwrap());
                assert!(json.starts_with("{\"kind\":"), "{json}");
            }
            let seam = unique(&mm.curve(209).unwrap());
            assert!(
                seam.starts_with(
                    r#"{"kind":"surface_curve","instance":209,"subtype":"seam_curve","curve_3d":{"kind":"circle""#
                ),
                "{seam}"
            );
            assert!(
                seam.ends_with(r#""master_representation":"pcurve_s1"}"#),
                "{seam}"
            );
            unique(&mm.vector(207).unwrap());
            unique(&mm.axis1_placement(210).unwrap());
            unique(&mm.axis2_placement_3d(104).unwrap().to_rigid());
        }

        #[test]
        fn errors_have_unique_keys() {
            let kinds = [
                GeometryErrorKind::Missing,
                GeometryErrorKind::WrongType { expected: "X" },
                GeometryErrorKind::Complex,
                GeometryErrorKind::ParameterCount {
                    expected: 2,
                    found: 3,
                },
                GeometryErrorKind::MissingAttribute,
                GeometryErrorKind::NotANumber,
                GeometryErrorKind::NotAReference,
                GeometryErrorKind::NotAList,
                GeometryErrorKind::NotFinite,
                GeometryErrorKind::Dimension { found: 2 },
                GeometryErrorKind::ZeroDirection,
                GeometryErrorKind::Parallel,
                GeometryErrorKind::OutOfRange { value: -1.0 },
                GeometryErrorKind::UnknownEnumeration,
                GeometryErrorKind::Unit(UnitError::Missing {
                    context: 4,
                    kind: UnitKind::Length,
                }),
            ];
            for kind in kinds {
                let error = GeometryError {
                    instance: 1,
                    entity: "X".to_owned(),
                    attribute: Some("radius"),
                    kind,
                };
                unique(&error);
                assert!(unique(&error.kind).starts_with("{\"kind\":"));
            }
            assert_eq!(
                unique(&GeometryErrorKind::Unit(UnitError::Missing {
                    context: 4,
                    kind: UnitKind::Length,
                })),
                r#"{"kind":"unit","error":{"kind":"missing","context":4,"unit_kind":"length"}}"#
            );
        }
    }
}
