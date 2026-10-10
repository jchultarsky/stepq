#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.10,<3.14"
# dependencies = ["cadquery-ocp==8.0.1.0.0"]
# ///
"""Geometric oracle for stepq: read STEP files through Open CASCADE and compare.

stepq reads only what files state, and a structurally broken STEP file can
read "successfully" with zero solids and no warning (docs/ARCHITECTURE.md,
"Failure is silent"). So stepq's outputs are checked here, at development
time only: read the input and every output through Open CASCADE and require
that the outputs together have the same number of solids, the same total
volume (to a relative 1e-9), and every product name and colour of the input.

    tools/verify-occt.py summary FILE...
    tools/verify-occt.py compare INPUT OUTPUT...

What stepq reads from a file is checked against Open CASCADE's reading by
tools/verify-read.py, which imports one function per check from here; each
is also a subcommand that prints Open CASCADE's side as tab-separated lines:

    tools/verify-occt.py units FILE...
    tools/verify-occt.py surfaces FILE...
    tools/verify-occt.py curves FILE...
    tools/verify-occt.py occurrences FILE...
    tools/verify-occt.py topology FILE...
    tools/verify-occt.py shapes FILE...

Exit status: 0 if every comparison holds, 1 if one fails, 2 if a file cannot
be read. Needs uv (https://docs.astral.sh/uv/), which installs cadquery-ocp
on first use.
"""

from __future__ import annotations

import argparse
import math
import sys
from dataclasses import dataclass, field
from pathlib import Path

REL_TOL = 1e-9
VOLUME_EPS = 1e-9


@dataclass
class Summary:
    """What Open CASCADE sees in one STEP file."""

    path: Path
    solids: int = 0
    volume: float = 0.0
    names: set[str] = field(default_factory=set)
    colours: set[tuple[float, float, float]] = field(default_factory=set)
    # Names of the top-level (free) shapes: the file's root products.
    top_names: set[str] = field(default_factory=set)
    # (solids, volume) of every part shape — each shape that is not an
    # assembly — measured unplaced, once per shape however often it is used.
    prototypes: list[tuple[int, float]] = field(default_factory=list)

    def line(self) -> str:
        return (
            f"{self.solids} solids, volume {self.volume:.6g}, "
            f"{len(self.names)} names, {len(self.colours)} colours"
        )


def read(path: Path) -> Summary:
    """Reads a STEP file with names and colours through XCAF."""
    from OCP.BRepGProp import BRepGProp
    from OCP.collections import Sequence_TDF_Label
    from OCP.GProp import GProp_GProps
    from OCP.IFSelect import IFSelect_RetDone
    from OCP.Quantity import Quantity_Color
    from OCP.STEPCAFControl import STEPCAFControl_Reader
    from OCP.TCollection import TCollection_ExtendedString
    from OCP.TDataStd import TDataStd_Name
    from OCP.TDocStd import TDocStd_Document
    from OCP.TopAbs import TopAbs_FACE, TopAbs_SOLID
    from OCP.TopExp import TopExp_Explorer
    from OCP.XCAFDoc import XCAFDoc_ColorType, XCAFDoc_DocumentTool

    document = TDocStd_Document(TCollection_ExtendedString("MDTV-XCAF"))
    reader = STEPCAFControl_Reader()
    reader.SetNameMode(True)
    reader.SetColorMode(True)
    if reader.ReadFile(str(path)) != IFSelect_RetDone or not reader.Transfer(document):
        raise OSError(f"Open CASCADE could not read {path}")

    shapes = XCAFDoc_DocumentTool.ShapeTool_s(document.Main())
    colours = XCAFDoc_DocumentTool.ColorTool_s(document.Main())
    summary = Summary(path)

    def labels(fill) -> list:
        sequence = Sequence_TDF_Label()
        fill(sequence)
        return [sequence.Value(i) for i in range(1, sequence.Length() + 1)]

    def colour_of(shape) -> None:
        colour = Quantity_Color()
        for kind in (XCAFDoc_ColorType.XCAFDoc_ColorSurf, XCAFDoc_ColorType.XCAFDoc_ColorGen):
            if colours.GetColor(shape, kind, colour):
                summary.colours.add(
                    (round(colour.Red(), 4), round(colour.Green(), 4), round(colour.Blue(), 4))
                )

    def name_of(label) -> str | None:
        # FindAttribute on a label without a name segfaults in cadquery-ocp
        # 8.0.1 (seen on moon_buggy_asm and nist_ctc_02_asme1_ap242-e2), so
        # ask first.
        if not label.IsAttribute(TDataStd_Name.GetID_s()):
            return None
        name = TDataStd_Name()
        label.FindAttribute(TDataStd_Name.GetID_s(), name)
        return name.Get().ToExtString()

    def volume_of(shape) -> float:
        # The sum over the shape's solids, each integrated on its own. One
        # integration over a whole compound is not additive: a compound that
        # also holds open shells (the surface models Creo writes next to a
        # solid) integrates to more than its solid, although each shell
        # alone integrates to 0, and differently once placed in an assembly.
        # Adaptive integration with an error bound: the default quadrature
        # differs by up to ~1e-6 relative between runs of the same geometry.
        total, explorer = 0.0, TopExp_Explorer(shape, TopAbs_SOLID)
        while explorer.More():
            properties = GProp_GProps()
            BRepGProp.VolumeProperties_s(explorer.Current(), properties, VOLUME_EPS)
            total += properties.Mass()
            explorer.Next()
        return total

    def solids_of(shape) -> int:
        count, explorer = 0, TopExp_Explorer(shape, TopAbs_SOLID)
        while explorer.More():
            count += 1
            explorer.Next()
        return count

    for label in labels(shapes.GetShapes):
        if (name := name_of(label)) is not None:
            summary.names.add(name)
        shape = shapes.GetShape_s(label)
        colour_of(shape)
        if not shapes.IsAssembly_s(label) and (count := solids_of(shape)):
            summary.prototypes.append((count, volume_of(shape)))

    for label in labels(shapes.GetFreeShapes):
        if (name := name_of(label)) is not None:
            summary.top_names.add(name)
        shape = shapes.GetShape_s(label)
        explorer = TopExp_Explorer(shape, TopAbs_SOLID)
        while explorer.More():
            summary.solids += 1
            colour_of(explorer.Current())
            explorer.Next()
        explorer = TopExp_Explorer(shape, TopAbs_FACE)
        while explorer.More():
            colour_of(explorer.Current())
            explorer.Next()
        summary.volume += volume_of(shape)
    return summary


def quiet() -> None:
    """Stops Open CASCADE printing its own messages (transfer statistics,
    reader warnings) between the lines a check prints."""
    from OCP.Message import Message

    messenger = Message.DefaultMessenger_s()
    printers = messenger.Printers()
    for index in range(printers.Length(), 0, -1):
        messenger.RemovePrinter(printers.Value(index))


@dataclass
class UnitContext:
    """Open CASCADE's reading of one representation context that assigns units."""

    # 1-based position of the context in the file: Open CASCADE's entity
    # number, which is stepq's node + 1.
    rank: int
    # Open CASCADE's class for the entity, such as
    # StepGeom_GeomRepContextAndGlobUnitAssCtxAndGlobUncertaintyAssCtx.
    entity: str
    # Factors to the millimetre, radian and steradian; None where Open
    # CASCADE finds no unit of that kind (LengthDone() and so on false).
    length: float | None
    plane_angle: float | None
    solid_angle: float | None
    # The smallest length uncertainty in millimetres; None if the context
    # states none Open CASCADE reads. Open CASCADE keeps the stated value, in
    # the uncertainty's own unit, and this multiplies it by the context's
    # length factor, which is right while the two units are the same (they
    # are in every fixture).
    uncertainty: float | None

    def line(self) -> str:
        values = (self.length, self.plane_angle, self.solid_angle, self.uncertainty)
        return "\t".join(
            [str(self.rank), self.entity, *("-" if v is None else repr(v) for v in values)]
        )


def units(path: Path) -> list[UnitContext]:
    """Every context of a file that assigns units, as Open CASCADE reads it.

    Open CASCADE's STEPConstruct_UnitContext is what its STEP reader uses to
    scale geometry: ComputeFactors over the context's units and
    ComputeTolerance over its uncertainties.
    """
    from OCP.IFSelect import IFSelect_RetDone
    from OCP.STEPConstruct import STEPConstruct_UnitContext
    from OCP.STEPControl import STEPControl_Reader
    from OCP.StepData import StepData_Factors

    reader = STEPControl_Reader()
    if reader.ReadFile(str(path)) != IFSelect_RetDone:
        raise OSError(f"Open CASCADE could not read {path}")
    model = reader.StepModel()
    contexts = []
    for rank in range(1, model.NbEntities() + 1):
        entity = model.Value(rank)
        if hasattr(entity, "GlobalUnitAssignedContext"):
            assigned = entity.GlobalUnitAssignedContext()
        elif hasattr(entity, "Units"):
            # A simple GLOBAL_UNIT_ASSIGNED_CONTEXT.
            assigned = entity
        else:
            continue
        context = STEPConstruct_UnitContext()
        context.ComputeFactors(assigned, StepData_Factors())
        uncertainty = None
        if hasattr(entity, "GlobalUncertaintyAssignedContext"):
            context.ComputeTolerance(entity.GlobalUncertaintyAssignedContext())
            if context.HasUncertainty():
                uncertainty = context.Uncertainty() * context.LengthFactor()
        contexts.append(
            UnitContext(
                rank=rank,
                entity=type(entity).__name__,
                length=context.LengthFactor() if context.LengthDone() else None,
                plane_angle=context.PlaneAngleFactor() if context.PlaneAngleDone() else None,
                solid_angle=context.SolidAngleFactor() if context.SolidAngleDone() else None,
                uncertainty=uncertainty,
            )
        )
    return contexts


def prototypes(path: Path) -> list:
    """The shape of every part in a file, unplaced and each once.

    These are XCAF's part prototypes: every shape label that is not an
    assembly, as read() takes them for its per-part volumes. A part placed
    many times is here once, in its own frame.
    """
    from OCP.collections import Sequence_TDF_Label
    from OCP.IFSelect import IFSelect_RetDone
    from OCP.STEPCAFControl import STEPCAFControl_Reader
    from OCP.TCollection import TCollection_ExtendedString
    from OCP.TDocStd import TDocStd_Document
    from OCP.XCAFDoc import XCAFDoc_DocumentTool

    document = TDocStd_Document(TCollection_ExtendedString("MDTV-XCAF"))
    reader = STEPCAFControl_Reader()
    if reader.ReadFile(str(path)) != IFSelect_RetDone or not reader.Transfer(document):
        raise OSError(f"Open CASCADE could not read {path}")
    shapes = XCAFDoc_DocumentTool.ShapeTool_s(document.Main())
    labels = Sequence_TDF_Label()
    shapes.GetShapes(labels)
    return [
        shapes.GetShape_s(labels.Value(i))
        for i in range(1, labels.Length() + 1)
        if not shapes.IsAssembly_s(labels.Value(i))
    ]


def _distinct(shapes: list, kind) -> list:
    """Every distinct sub-shape of `kind` in `shapes`, each once however many
    faces, shells or parts share it (orientation ignored)."""
    from OCP.collections import IndexedMap_TopoDS_Shape_TopTools_ShapeMapHasher as Map
    from OCP.TopExp import TopExp

    found = Map()
    for shape in shapes:
        TopExp.MapShapes_s(shape, kind, found)
    return [found.FindKey(i) for i in range(1, found.Extent() + 1)]


def _xyz(point) -> tuple[float, float, float]:
    return (point.X(), point.Y(), point.Z())


@dataclass
class Geometry:
    """Open CASCADE's reading of one face's surface or one edge's curve."""

    # plane, cylinder, cone, sphere, torus, line or circle; for anything
    # else, Open CASCADE's type, such as GeomAbs_BSplineSurface.
    kind: str
    # The position's location (a line's point), in mm.
    location: tuple[float, float, float] = (0.0, 0.0, 0.0)
    # The position's z axis (a line's direction), as Open CASCADE keeps it.
    axis: tuple[float, float, float] = (0.0, 0.0, 0.0)
    # Radius (a torus's major radius, a cone's reference radius), in mm.
    a: float = 0.0
    # A cone's semi-angle in radians, a torus's minor radius in mm.
    b: float = 0.0
    # An edge only: True if it is a seam of one of its faces (that face
    # uses it twice, with two curves on its surface).
    seam: bool = False

    def line(self) -> str:
        values = (*self.location, *self.axis, self.a, self.b)
        return "\t".join([self.kind, *(repr(v) for v in values), "seam" if self.seam else "-"])


def surfaces(path: Path) -> list[Geometry]:
    """The surface of every distinct face of every part prototype, as
    BRepAdaptor_Surface gives it, in mm and rad. Tessellated faces, which
    have no surface, are left out."""
    from OCP.BRep import BRep_Tool
    from OCP.BRepAdaptor import BRepAdaptor_Surface
    from OCP.GeomAbs import GeomAbs_SurfaceType as Type
    from OCP.TopAbs import TopAbs_FACE
    from OCP.TopoDS import TopoDS

    found = []
    for shape in _distinct(prototypes(path), TopAbs_FACE):
        face = TopoDS.Face(shape)
        if not BRep_Tool.IsGeometric_s(face):
            # A tessellated face: a triangulation, no surface.
            continue
        surface = BRepAdaptor_Surface(face, False)
        kind = surface.GetType()
        if kind == Type.GeomAbs_Plane:
            g = surface.Plane()
            found.append(Geometry("plane", _xyz(g.Location()), _xyz(g.Axis().Direction())))
        elif kind == Type.GeomAbs_Cylinder:
            g = surface.Cylinder()
            found.append(
                Geometry("cylinder", _xyz(g.Location()), _xyz(g.Axis().Direction()), g.Radius())
            )
        elif kind == Type.GeomAbs_Cone:
            g = surface.Cone()
            found.append(
                Geometry(
                    "cone",
                    _xyz(g.Location()),
                    _xyz(g.Axis().Direction()),
                    g.RefRadius(),
                    g.SemiAngle(),
                )
            )
        elif kind == Type.GeomAbs_Sphere:
            g = surface.Sphere()
            found.append(
                Geometry("sphere", _xyz(g.Location()), _xyz(g.Position().Direction()), g.Radius())
            )
        elif kind == Type.GeomAbs_Torus:
            g = surface.Torus()
            found.append(
                Geometry(
                    "torus",
                    _xyz(g.Location()),
                    _xyz(g.Axis().Direction()),
                    g.MajorRadius(),
                    g.MinorRadius(),
                )
            )
        else:
            found.append(Geometry(kind.name))
    return found


def curves(path: Path) -> list[Geometry]:
    """The 3D curve of every distinct edge of a face of every part
    prototype, as BRepAdaptor_Curve gives it, in mm, and whether it is a
    seam. Edges that bound no face (the curves of a wireframe or curve set,
    which Open CASCADE also reads as edges), degenerated edges, which have
    no 3D curve and which STEP states as no edge_curve, and the edges of
    tessellated faces, which have no curve at all, are left out.

    Open CASCADE heals while it reads, so its edges are not only the file's:
    a periodic face that the file bounds only by closed loops (a cylinder
    between two circles, with no seam edge) gets a seam, and the closed
    edges are split where the seam meets them.
    """
    from OCP.BRep import BRep_Tool
    from OCP.BRepAdaptor import BRepAdaptor_Curve
    from OCP.GeomAbs import GeomAbs_CurveType as Type
    from OCP.TopAbs import TopAbs_EDGE, TopAbs_FACE
    from OCP.TopExp import TopExp_Explorer
    from OCP.TopoDS import TopoDS
    from OCP.collections import IndexedMap_TopoDS_Shape_TopTools_ShapeMapHasher as Map

    edges, seams = Map(), set()
    for shape in _distinct(prototypes(path), TopAbs_FACE):
        face = TopoDS.Face(shape)
        explorer = TopExp_Explorer(face, TopAbs_EDGE)
        while explorer.More():
            edge = TopoDS.Edge(explorer.Current())
            index = edges.Add(edge)
            if BRep_Tool.IsClosed_s(edge, face):
                seams.add(index)
            explorer.Next()

    found = []
    for index in range(1, edges.Extent() + 1):
        edge = TopoDS.Edge(edges.FindKey(index))
        if BRep_Tool.Degenerated_s(edge) or not BRep_Tool.IsGeometric_s(edge):
            continue
        curve = BRepAdaptor_Curve(edge)
        kind = curve.GetType()
        seam = index in seams
        if kind == Type.GeomAbs_Line:
            g = curve.Line()
            found.append(Geometry("line", _xyz(g.Location()), _xyz(g.Direction()), seam=seam))
        elif kind == Type.GeomAbs_Circle:
            g = curve.Circle()
            found.append(
                Geometry(
                    "circle",
                    _xyz(g.Location()),
                    _xyz(g.Axis().Direction()),
                    g.Radius(),
                    seam=seam,
                )
            )
        else:
            found.append(Geometry(kind.name, seam=seam))
    return found


@dataclass
class Occurrence:
    """Open CASCADE's reading of one occurrence: a shape label reached from a
    free shape through component labels, with its location composed from the
    root down."""

    # The name of each component label on the path from the free shape, the
    # free shape itself first (its own name); '?' where a label has none.
    # Open CASCADE names a component after its NAUO: the description if not
    # blank, else the name if not blank, else the id.
    instances: tuple[str, ...]
    # The name of the shape each label refers to: the product name, or its
    # id if the name is blank; '?' if none. Open CASCADE also lists the
    # solids, shells and compounds of a part's shape as labels of their own,
    # named SOLID, SHELL or COMPOUND (or unnamed), which are not occurrences.
    products: tuple[str, ...]
    # The 3x4 transform to the free shape's frame, row by row, in mm.
    matrix: tuple[float, ...]

    def line(self) -> str:
        return "\t".join(
            ["/".join(self.instances), "/".join(self.products), *(repr(v) for v in self.matrix)]
        )


def occurrences(path: Path) -> list[Occurrence]:
    """Every occurrence of every free shape, depth first, as XCAF holds them.

    Each free shape (from GetFreeShapes) is a root; each component of an
    assembly label (GetComponents, not recursively) leads to the label it
    refers to (GetReferredShape), whose location (GetLocation) composes with
    its parent's (TopLoc_Location.Multiplied). Open CASCADE reads lengths in
    millimetres.
    """
    from OCP.collections import Sequence_TDF_Label
    from OCP.IFSelect import IFSelect_RetDone
    from OCP.STEPCAFControl import STEPCAFControl_Reader
    from OCP.TCollection import TCollection_ExtendedString
    from OCP.TDataStd import TDataStd_Name
    from OCP.TDF import TDF_Label
    from OCP.TDocStd import TDocStd_Document
    from OCP.XCAFDoc import XCAFDoc_DocumentTool

    document = TDocStd_Document(TCollection_ExtendedString("MDTV-XCAF"))
    reader = STEPCAFControl_Reader()
    reader.SetNameMode(True)
    if reader.ReadFile(str(path)) != IFSelect_RetDone or not reader.Transfer(document):
        raise OSError(f"Open CASCADE could not read {path}")
    shapes = XCAFDoc_DocumentTool.ShapeTool_s(document.Main())

    def name_of(label) -> str:
        # Asked first: FindAttribute on a label without a name segfaults in
        # cadquery-ocp 8.0.1 (see read()).
        if not label.IsAttribute(TDataStd_Name.GetID_s()):
            return "?"
        name = TDataStd_Name()
        label.FindAttribute(TDataStd_Name.GetID_s(), name)
        return name.Get().ToExtString()

    def matrix(location) -> tuple[float, ...]:
        transform = location.Transformation()
        return tuple(transform.Value(row, col) for row in (1, 2, 3) for col in (1, 2, 3, 4))

    found = []
    free = Sequence_TDF_Label()
    shapes.GetFreeShapes(free)
    for i in range(1, free.Length() + 1):
        label = free.Value(i)
        name = name_of(label)
        # An explicit stack, so that depth is not bounded by Python's.
        stack = [(label, (name,), (name,), shapes.GetShape_s(label).Location())]
        while stack:
            label, instances, products, location = stack.pop()
            found.append(Occurrence(instances, products, matrix(location)))
            if not shapes.IsAssembly_s(label):
                continue
            components = Sequence_TDF_Label()
            shapes.GetComponents_s(label, components, False)
            children = []
            for j in range(1, components.Length() + 1):
                component = components.Value(j)
                referred = TDF_Label()
                if not shapes.GetReferredShape_s(component, referred):
                    continue
                children.append(
                    (
                        referred,
                        (*instances, name_of(component)),
                        (*products, name_of(referred)),
                        location.Multiplied(shapes.GetLocation_s(component)),
                    )
                )
            stack.extend(reversed(children))
    return found


@dataclass
class Topology:
    """Open CASCADE's reading of one solid's topology. Faces, edges and
    vertices are named by entity number (the 1-based position in the file
    of the entity Open CASCADE read them from); faces and edges it made
    itself, or bound to no entity, are counted in `unbound`."""

    # The solid's entity number, and the class Open CASCADE read it as,
    # such as StepShape_ManifoldSolidBrep.
    rank: int
    entity: str
    # Each face's entity number, and whether it is REVERSED in the solid
    # (its orientation composed with its shell's).
    reversed: dict[int, bool]
    # Each edge's entity number, and the entity numbers of the faces that
    # use it.
    faces_of_edge: dict[int, list[int]]
    # Each edge's entity number, and the entity number of its first vertex
    # by parameter (None if that vertex is bound to no entity).
    first_vertex: dict[int, int | None]
    # (face, edge, FORWARD) for each use of a bound edge by a bound face:
    # the edge's orientation in the face as the solid holds it.
    uses: list[tuple[int, int, bool]]
    # (face, edge) for each edge closed in a face: a seam.
    closed: list[tuple[int, int]]
    # Faces and edges bound to no entity: made by healing.
    unbound: int

    def line(self) -> str:
        """Tab-separated: rank, faces, edges, seams, unbound sub-shapes."""
        return "\t".join(
            str(value)
            for value in (
                self.rank,
                len(self.reversed),
                len(self.faces_of_edge),
                len(self.closed),
                self.unbound,
            )
        )


def topology(path: Path) -> list[Topology]:
    """Every solid Open CASCADE reads from a file's root transfers, once
    each, with its faces, edges and vertices bound back to the entities
    they were read from (TransferReader.EntityFromShapeResult on the
    unplaced shape). Its STEP reader heals as it reads, so it may hold
    edges and vertices the file does not state."""
    from OCP.BRep import BRep_Tool
    from OCP.collections import (
        IndexedDataMap_TopoDS_Shape_List_TopoDS_Shape_TopTools_ShapeMapHasher as Ancestors,
    )
    from OCP.IFSelect import IFSelect_RetDone
    from OCP.STEPControl import STEPControl_Reader
    from OCP.TopAbs import TopAbs_EDGE, TopAbs_FACE, TopAbs_FORWARD, TopAbs_REVERSED
    from OCP.TopAbs import TopAbs_SOLID
    from OCP.TopExp import TopExp, TopExp_Explorer
    from OCP.TopLoc import TopLoc_Location
    from OCP.TopoDS import TopoDS

    reader = STEPControl_Reader()
    if reader.ReadFile(str(path)) != IFSelect_RetDone:
        raise OSError(f"Open CASCADE could not read {path}")
    model = reader.StepModel()
    # Number() returns 0 through these bindings, so entities are told apart
    # by identity; the list keeps each binding alive.
    entities = [model.Value(rank) for rank in range(1, model.NbEntities() + 1)]
    ranks = {id(entity): rank for rank, entity in enumerate(entities, 1)}
    reader.TransferRoots()
    transfers = reader.WS().TransferReader()

    # EntityFromShapeResult searches every transfer result, so each shape
    # is looked up once: unplaced and forward, as the transfer bound it.
    looked_up: dict = {}

    def rank(shape) -> int | None:
        key = shape.Located(TopLoc_Location()).Oriented(TopAbs_FORWARD)
        if key not in looked_up:
            entity = transfers.EntityFromShapeResult(key, 1)
            looked_up[key] = None if entity is None else ranks.get(id(entity))
        return looked_up[key]

    def explore(shape, kind):
        explorer = TopExp_Explorer(shape, kind)
        while explorer.More():
            yield explorer.Current()
            explorer.Next()

    solids, seen = [], set()
    for index in range(1, reader.NbShapes() + 1):
        for found in explore(reader.Shape(index), TopAbs_SOLID):
            solid = found.Located(TopLoc_Location())
            solid_rank = rank(solid)
            if solid_rank is None or solid_rank in seen:
                continue
            seen.add(solid_rank)
            unbound = 0
            reversed_, uses, closed = {}, [], []
            for shape in explore(solid, TopAbs_FACE):
                face = TopoDS.Face(shape)
                face_rank = rank(face)
                if face_rank is None:
                    unbound += 1
                    continue
                reversed_[face_rank] = face.Orientation() == TopAbs_REVERSED
                # Explored from the face as the solid holds it, so each
                # edge's orientation is composed with the face's.
                for edge_shape in explore(face, TopAbs_EDGE):
                    edge = TopoDS.Edge(edge_shape)
                    edge_rank = rank(edge)
                    if edge_rank is None:
                        continue
                    uses.append((face_rank, edge_rank, edge.Orientation() == TopAbs_FORWARD))
                    if BRep_Tool.IsClosed_s(edge, face):
                        closed.append((face_rank, edge_rank))
            ancestors = Ancestors()
            TopExp.MapShapesAndAncestors_s(solid, TopAbs_EDGE, TopAbs_FACE, ancestors)
            faces_of_edge, first_vertex = {}, {}
            for i in range(1, ancestors.Extent() + 1):
                edge = TopoDS.Edge(ancestors.FindKey(i))
                edge_rank = rank(edge)
                if edge_rank is None:
                    unbound += 1
                    continue
                faces_of_edge[edge_rank] = sorted(
                    {r for r in (rank(face) for face in ancestors.FindFromIndex(i)) if r}
                )
                along = TopoDS.Edge(edge.Oriented(TopAbs_FORWARD))
                first_vertex[edge_rank] = rank(TopExp.FirstVertex_s(along))
            solids.append(
                Topology(
                    rank=solid_rank,
                    entity=type(entities[solid_rank - 1]).__name__,
                    reversed=reversed_,
                    faces_of_edge=faces_of_edge,
                    first_vertex=first_vertex,
                    uses=uses,
                    closed=closed,
                    unbound=unbound,
                )
            )
    return solids


@dataclass
class LabelShape:
    """One shape label of Open CASCADE's XCAF document."""

    # Its name: the product's name, else its id; None if it has none.
    name: str | None
    # The solids of its shape, a component placed n times counted n times.
    solids: int

    def line(self) -> str:
        """Tab-separated: name ('?' if none), solids."""
        return f"{self.name if self.name is not None else '?'}\t{self.solids}"


def shapes(path: Path) -> list[LabelShape]:
    """Every shape label of a file, assemblies included, with the solids of
    its shape (TopExp_Explorer over GetShape)."""
    from OCP.collections import Sequence_TDF_Label
    from OCP.IFSelect import IFSelect_RetDone
    from OCP.STEPCAFControl import STEPCAFControl_Reader
    from OCP.TCollection import TCollection_ExtendedString
    from OCP.TDataStd import TDataStd_Name
    from OCP.TDocStd import TDocStd_Document
    from OCP.TopAbs import TopAbs_SOLID
    from OCP.TopExp import TopExp_Explorer
    from OCP.XCAFDoc import XCAFDoc_DocumentTool

    document = TDocStd_Document(TCollection_ExtendedString("MDTV-XCAF"))
    reader = STEPCAFControl_Reader()
    reader.SetNameMode(True)
    if reader.ReadFile(str(path)) != IFSelect_RetDone or not reader.Transfer(document):
        raise OSError(f"Open CASCADE could not read {path}")
    tool = XCAFDoc_DocumentTool.ShapeTool_s(document.Main())
    labels = Sequence_TDF_Label()
    tool.GetShapes(labels)
    found = []
    for index in range(1, labels.Length() + 1):
        label = labels.Value(index)
        name = None
        # Asked first: FindAttribute on a label without a name segfaults in
        # cadquery-ocp 8.0.1 (see read()).
        if label.IsAttribute(TDataStd_Name.GetID_s()):
            attribute = TDataStd_Name()
            label.FindAttribute(TDataStd_Name.GetID_s(), attribute)
            name = attribute.Get().ToExtString()
        explorer = TopExp_Explorer(tool.GetShape_s(label), TopAbs_SOLID)
        solids = 0
        while explorer.More():
            solids += 1
            explorer.Next()
        found.append(LabelShape(name=name, solids=solids))
    return found


def compare(
    source: Summary, outputs: list[Summary], ignore_names: frozenset[str] = frozenset()
) -> list[str]:
    """The ways `outputs` together fail to account for `source`.

    `ignore_names` lists names allowed to be missing, such as an assembly's
    own name when its components are compared against it.
    """
    problems = []
    solids = sum(o.solids for o in outputs)
    volume = sum(o.volume for o in outputs)
    if solids != source.solids:
        problems.append(f"solids: input has {source.solids}, outputs have {solids}")
    if not math.isclose(volume, source.volume, rel_tol=REL_TOL, abs_tol=0.0):
        scale = max(abs(source.volume), abs(volume)) or 1.0
        problems.append(
            f"volume: input {source.volume!r}, outputs {volume!r} "
            f"(relative difference {abs(volume - source.volume) / scale:.3e})"
        )
    names = set().union(*(o.names for o in outputs))
    if missing := sorted(source.names - names - ignore_names):
        problems.append(f"names missing from outputs: {missing}")
    colours = set().union(*(o.colours for o in outputs))
    if missing := sorted(source.colours - colours):
        problems.append(f"colours missing from outputs: {missing}")
    return problems


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    commands = parser.add_subparsers(dest="command", required=True)
    summary = commands.add_parser("summary", help="print what Open CASCADE reads")
    summary.add_argument("files", nargs="+", type=Path)
    check = commands.add_parser("compare", help="check outputs against an input")
    check.add_argument("input", type=Path)
    check.add_argument("outputs", nargs="+", type=Path)
    unit_contexts = commands.add_parser(
        "units",
        help="print each context's unit factors and uncertainty: "
        "file, rank, entity, length, plane angle, solid angle, uncertainty (mm); '-' if none",
    )
    unit_contexts.add_argument("files", nargs="+", type=Path)
    occurrence_list = commands.add_parser(
        "occurrences",
        help="print every occurrence of every free shape: file, instance-name path, "
        "product-name path, then the 3x4 transform to the root row by row, in mm",
    )
    occurrence_list.add_argument("files", nargs="+", type=Path)
    solid_list = commands.add_parser(
        "topology",
        help="print each solid Open CASCADE reads: file, the solid's entity number, faces, "
        "edges, seams, and faces and edges bound to no entity",
    )
    solid_list.add_argument("files", nargs="+", type=Path)
    label_list = commands.add_parser(
        "shapes",
        help="print each shape label, assemblies included: file, name, solids of its shape",
    )
    label_list.add_argument("files", nargs="+", type=Path)
    for name, function, what in (
        ("surfaces", surfaces, "face"),
        ("curves", curves, "edge"),
    ):
        command = commands.add_parser(
            name,
            help=f"print the geometry of each distinct {what} of each part, unplaced: file, kind, "
            "location (3), axis (3), radius, semi-angle or minor radius, 'seam' or '-'; mm and rad",
        )
        command.add_argument("files", nargs="+", type=Path)
        command.set_defaults(function=function)
    args = parser.parse_args()

    try:
        if args.command == "units":
            for path in args.files:
                for context in units(path):
                    print(f"{path}\t{context.line()}")
            return 0
        if args.command == "occurrences":
            for path in args.files:
                for occurrence in occurrences(path):
                    print(f"{path}\t{occurrence.line()}")
            return 0
        if args.command == "shapes":
            for path in args.files:
                for label in shapes(path):
                    print(f"{path}\t{label.line()}")
            return 0
        if args.command == "topology":
            for path in args.files:
                for solid in topology(path):
                    print(f"{path}\t{solid.line()}")
            return 0
        if args.command in ("surfaces", "curves"):
            for path in args.files:
                for geometry in args.function(path):
                    print(f"{path}\t{geometry.line()}")
            return 0
        if args.command == "summary":
            for path in args.files:
                result = read(path)
                print(f"{path}: {result.line()}")
                print(f"  names: {sorted(result.names)}")
                print(f"  colours: {sorted(result.colours)}")
            return 0

        source = read(args.input)
        outputs = [read(path) for path in args.outputs]
    except OSError as error:
        print(f"error: {error}", file=sys.stderr)
        return 2

    problems = compare(source, outputs)
    if source.solids == 0 and not problems:
        print(f"WARN {args.input}: Open CASCADE reads no solids; only emptiness was compared")
        return 0
    if problems:
        print(f"FAIL {args.input}: {source.line()}")
        for problem in problems:
            print(f"  {problem}")
        return 1
    print(f"OK   {args.input}: {source.line()}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
