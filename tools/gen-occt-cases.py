#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.10,<3.14"
# dependencies = ["cadquery-ocp==8.0.1.0.0"]
# ///
"""Write the STEP files only Open CASCADE can write, for tools/verify-read.py.

These are generated where they are checked (CI's geometry job) and never
committed: they come from the pinned cadquery-ocp, so the same version
always writes the same files.

    tools/gen-occt-cases.py OUT_DIR

Writes, into OUT_DIR:

  plate_mm.step, plate_inch.step, plate_ft.step, plate_cm.step, plate_m.step
      One part, a 40 x 30 x 8 mm plate with a 6.6 mm hole, written with
      write.step.unit set to MM, INCH, FT, CM and M. The same geometry in
      five length units: Open CASCADE states the inch and the foot as
      conversion-based units over the millimetre, and the others as SI
      metres with and without a prefix, so the plates must all read the
      same in millimetres.

  cone_void_mm.step, cone_void_inch.step
      One part with every elementary surface: a 60 x 40 x 30 mm block with
      a blind hole that ends in a 118-degree drill point (a cone point), a
      toroidal groove around a boss in its top, and a spherical void, so
      Open CASCADE writes it as a brep_with_voids. Written in millimetres
      and inches; every edge is a surface or seam curve over a line or
      circle, with .PCURVE_S1. as its master, as Open CASCADE writes them.

and, into OUT_DIR/assemblies, for the occurrences check (CI runs the surface
and curve checks on the parts alone, whose counts it pins):

  nested_asm.step
      A nested assembly as cadquery writes one (XCAF and
      STEPCAFControl_Writer): ASM holds a PLATE and the sub-assembly
      BOLT_PAIR twice, each BOLT_PAIR two BOLTs, every component turned
      about an arbitrary axis and moved; 8 occurrences.

  divergent_both_frames.step
      nested_asm.step with the PLATE's item_defined_transformation given a
      component-side frame that is not the identity: a frame at (10, 0, 0)
      turned onto x, added to the PLATE's representation. ISO 10303 and
      Open CASCADE compose such a pair differently.

  divergent_defaulted_x.step
      nested_asm.step with the assembly-side frame of the first BOLT_PAIR
      replaced by one on axis (0, 0, -1) with no ref_direction, at the same
      location: the schema's default x is (1, 0, 0), Open CASCADE's
      (-1, 0, 0).

Later read checks add their own cases here.
"""

from __future__ import annotations

import sys
from pathlib import Path

# write.step.unit values; Open CASCADE 8.0.1 accepts "INCH", not "IN".
PLATE_UNITS = ("MM", "INCH", "FT", "CM", "M")


def plate():
    """The part the plates hold: a box with a hole through it."""
    from OCP.BRepAlgoAPI import BRepAlgoAPI_Cut
    from OCP.BRepPrimAPI import BRepPrimAPI_MakeBox, BRepPrimAPI_MakeCylinder
    from OCP.gp import gp_Ax2, gp_Dir, gp_Pnt

    box = BRepPrimAPI_MakeBox(40.0, 30.0, 8.0).Shape()
    hole = BRepPrimAPI_MakeCylinder(gp_Ax2(gp_Pnt(20, 15, -1), gp_Dir(0, 0, 1)), 3.3, 10.0).Shape()
    return BRepAlgoAPI_Cut(box, hole).Shape()


def cone_void():
    """The part with every elementary surface, a cone point and a void."""
    import math

    from OCP.BRepAlgoAPI import BRepAlgoAPI_Cut, BRepAlgoAPI_Fuse
    from OCP.BRepPrimAPI import (
        BRepPrimAPI_MakeBox,
        BRepPrimAPI_MakeCone,
        BRepPrimAPI_MakeCylinder,
        BRepPrimAPI_MakeSphere,
        BRepPrimAPI_MakeTorus,
    )
    from OCP.gp import gp_Ax2, gp_Dir, gp_Pnt

    block = BRepPrimAPI_MakeBox(60.0, 40.0, 30.0).Shape()
    # A blind hole of radius 4 down to z = 12, ending in a drill point.
    bore = BRepPrimAPI_MakeCylinder(gp_Ax2(gp_Pnt(15, 20, 12), gp_Dir(0, 0, 1)), 4.0, 20.0).Shape()
    depth = 4.0 / math.tan(math.radians(59.0))
    point = BRepPrimAPI_MakeCone(
        gp_Ax2(gp_Pnt(15, 20, 12), gp_Dir(0, 0, -1)), 4.0, 0.0, depth
    ).Shape()
    part = BRepAlgoAPI_Cut(block, BRepAlgoAPI_Fuse(bore, point).Shape()).Shape()
    # A toroidal groove around (42, 20) in the top face.
    groove = BRepPrimAPI_MakeTorus(gp_Ax2(gp_Pnt(42, 20, 30), gp_Dir(0, 0, 1)), 8.0, 2.0).Shape()
    part = BRepAlgoAPI_Cut(part, groove).Shape()
    # A spherical void well inside the block.
    void = BRepPrimAPI_MakeSphere(gp_Pnt(42, 20, 12), 5.0).Shape()
    return BRepAlgoAPI_Cut(part, void).Shape()


def nested_assembly():
    """An XCAF document holding the nested assembly, its root last."""
    from OCP.BRepPrimAPI import BRepPrimAPI_MakeBox, BRepPrimAPI_MakeCylinder
    from OCP.gp import gp_Ax1, gp_Dir, gp_Pnt, gp_Trsf, gp_Vec
    from OCP.TCollection import TCollection_ExtendedString
    from OCP.TDataStd import TDataStd_Name
    from OCP.TDocStd import TDocStd_Document
    from OCP.TopLoc import TopLoc_Location
    from OCP.XCAFDoc import XCAFDoc_DocumentTool

    def place(axis, angle, move):
        turn = gp_Trsf()
        turn.SetRotation(gp_Ax1(gp_Pnt(0, 0, 0), gp_Dir(*axis)), angle)
        shift = gp_Trsf()
        shift.SetTranslation(gp_Vec(*move))
        return TopLoc_Location(shift.Multiplied(turn))

    document = TDocStd_Document(TCollection_ExtendedString("XmlOcaf"))
    shapes = XCAFDoc_DocumentTool.ShapeTool_s(document.Main())

    def named(label, name):
        TDataStd_Name.Set_s(label, TCollection_ExtendedString(name))
        return label

    plate = named(shapes.AddShape(BRepPrimAPI_MakeBox(100, 60, 10).Shape(), False), "PLATE")
    bolt = named(shapes.AddShape(BRepPrimAPI_MakeCylinder(3, 18).Shape(), False), "BOLT")
    pair = named(shapes.NewShape(), "BOLT_PAIR")
    shapes.AddComponent(pair, bolt, place((0, 0, 1), 0.0, (-30, -15, 0)))
    shapes.AddComponent(pair, bolt, place((1, 1, 0), 0.6458, (30.5, 15.25, -2)))
    top = named(shapes.NewShape(), "ASM")
    shapes.AddComponent(top, plate, place((0, 0, 1), 0.0, (0, 0, -10)))
    shapes.AddComponent(top, pair, place((0.3, -0.2, 0.9), 2.1, (200, -40, 17)))
    shapes.AddComponent(top, pair, place((0, 0, -1), 3.14159, (-5, 7, 0)))
    shapes.UpdateAssemblies()
    return document


class Part21:
    """The instances of a Part 21 file, for editing a few of them by text.

    Only what the divergence cases need: instances are split at top-level
    semicolons (outside strings), and an instance's attributes, or a partial
    entity's, are split at top-level commas. Edited and added instances are
    written back in file order, the rest unchanged.
    """

    def __init__(self, text: str):
        start = text.index("DATA;") + len("DATA;")
        self.head, self.instances, self.tail = text[:start], {}, ""
        chunk, depth, quoted, i = [], 0, False, start
        while i < len(text):
            c = text[i]
            if c == "'":
                quoted = not quoted
            elif not quoted and c == ";":
                statement = "".join(chunk).strip()
                chunk = []
                if statement == "ENDSEC":
                    self.tail = text[i + 1:]
                    break
                name, _, value = statement.partition("=")
                self.instances[int(name.strip()[1:])] = " ".join(value.split())
                i += 1
                continue
            chunk.append(c)
            i += 1

    def text(self) -> str:
        body = "".join(f"\n#{i} = {value};" for i, value in self.instances.items())
        return f"{self.head}{body}\nENDSEC;{self.tail}"

    def add(self, value: str) -> int:
        number = max(self.instances) + 1
        self.instances[number] = value
        return number

    @staticmethod
    def split(inner: str) -> list[str]:
        """Splits at top-level commas."""
        parts, depth, quoted, current = [], 0, False, []
        for c in inner:
            if c == "'":
                quoted = not quoted
            elif not quoted and c == "(":
                depth += 1
            elif not quoted and c == ")":
                depth -= 1
            elif not quoted and depth == 0 and c == ",":
                parts.append("".join(current).strip())
                current = []
                continue
            current.append(c)
        parts.append("".join(current).strip())
        return parts

    def attributes(self, number: int, partial: str | None = None) -> tuple[str, list[str]]:
        """The entity name and attributes of a simple instance, or of one
        partial entity of a complex one."""
        value = self.instances[number]
        if partial is not None:
            at = value.index(partial + "(") + len(partial)
            value = value[at - len(partial):]
        name, _, rest = value.partition("(")
        depth, end = 1, 0
        for end, c in enumerate(rest):
            depth += {"(": 1, ")": -1}.get(c, 0)
            if depth == 0:
                break
        return name.strip(), self.split(rest[:end])

    def set_attributes(self, number: int, attributes: list[str]) -> None:
        name, _ = self.attributes(number)
        self.instances[number] = f"{name}({','.join(attributes)})"

    def find(self, entity: str, index: int, value: str) -> int:
        """The first instance of `entity` whose attribute `index` is `value`."""
        for number, text in self.instances.items():
            if text.startswith(entity + "(") and self.attributes(number)[1][index] == value:
                return number
        raise LookupError(f"no {entity} with attribute {index} = {value}")

    def placement(self, product: str) -> tuple[int, int, int, int]:
        """For the first usage of the product named `product`: its NAUO, its
        item_defined_transformation, and the relationship's rep_1 and
        rep_2."""
        found = self.find("PRODUCT", 1, f"'{product}'")
        formation = next(
            n for n, t in self.instances.items()
            if t.startswith("PRODUCT_DEFINITION_FORMATION")
            and self.attributes(n)[1][2] == f"#{found}"
        )
        definition = self.find("PRODUCT_DEFINITION", 2, f"#{formation}")
        usage = self.find("NEXT_ASSEMBLY_USAGE_OCCURRENCE", 4, f"#{definition}")
        shape = self.find("PRODUCT_DEFINITION_SHAPE", 2, f"#{usage}")
        link = self.find("CONTEXT_DEPENDENT_SHAPE_REPRESENTATION", 1, f"#{shape}")
        relationship = int(self.attributes(link)[1][0][1:])
        _, (transformation,) = self.attributes(
            relationship, "REPRESENTATION_RELATIONSHIP_WITH_TRANSFORMATION"
        )
        _, related = self.attributes(relationship, "REPRESENTATION_RELATIONSHIP")
        return usage, int(transformation[1:]), int(related[2][1:]), int(related[3][1:])

    def frame(self, location: str, axis: str | None, ref_direction: str | None) -> int:
        """A new axis2_placement_3d; directions of None are left unset."""
        point = self.add(f"CARTESIAN_POINT('',{location})")
        directions = []
        for ratios in (axis, ref_direction):
            if ratios is None:
                directions.append("$")
            else:
                direction = self.add("DIRECTION(''," + ratios + ")")
                directions.append(f"#{direction}")
        return self.add(f"AXIS2_PLACEMENT_3D('',#{point},{directions[0]},{directions[1]})")

    def list_item(self, representation: int, item: int) -> None:
        """Adds `item` to a representation's items."""
        _, attributes = self.attributes(representation)
        attributes[1] = attributes[1][:-1] + f",#{item})"
        self.set_attributes(representation, attributes)


def both_frames(asm: Part21) -> None:
    """Gives the PLATE's transformation a component-side frame that is not
    the identity."""
    _, transformation, rep_1, _ = asm.placement("PLATE")
    frame = asm.frame("(10.,0.,0.)", "(1.,0.,0.)", "(0.,1.,0.)")
    asm.list_item(rep_1, frame)
    _, attributes = asm.attributes(transformation)
    attributes[2] = f"#{frame}"
    asm.set_attributes(transformation, attributes)


def defaulted_x(asm: Part21) -> None:
    """Puts the first BOLT_PAIR on a frame on axis (0, 0, -1) with no
    ref_direction, where the old frame was."""
    _, transformation, _, rep_2 = asm.placement("BOLT_PAIR")
    _, attributes = asm.attributes(transformation)
    old = int(attributes[3][1:])
    point = asm.attributes(old)[1][1]
    _, coordinates = asm.attributes(int(point[1:]))
    frame = asm.frame(coordinates[1], "(0.,0.,-1.)", None)
    asm.list_item(rep_2, frame)
    attributes[3] = f"#{frame}"
    asm.set_attributes(transformation, attributes)


def write_assemblies(out: Path) -> list[Path]:
    """Writes the nested assembly and the two divergence cases made from it."""
    from OCP.IFSelect import IFSelect_RetDone
    from OCP.Interface import Interface_Static
    from OCP.STEPCAFControl import STEPCAFControl_Writer
    from OCP.STEPControl import STEPControl_AsIs

    out.mkdir(parents=True, exist_ok=True)
    nested = out / "nested_asm.step"
    writer = STEPCAFControl_Writer()
    # write.step.unit is global: the parts may have left it at INCH.
    if not Interface_Static.SetCVal_s("write.step.unit", "MM"):
        raise RuntimeError("Open CASCADE does not accept write.step.unit = MM")
    if not writer.Transfer(nested_assembly(), STEPControl_AsIs):
        raise RuntimeError("Open CASCADE could not transfer the nested assembly")
    if writer.Write(str(nested)) != IFSelect_RetDone:
        raise OSError(f"Open CASCADE could not write {nested}")
    written = [nested]
    text = nested.read_text(encoding="latin-1")
    for name, edit in (("divergent_both_frames", both_frames), ("divergent_defaulted_x", defaulted_x)):
        asm = Part21(text)
        edit(asm)
        path = out / f"{name}.step"
        path.write_text(asm.text(), encoding="latin-1")
        written.append(path)
    return written


def quiet() -> None:
    """Stops Open CASCADE printing transfer statistics for every file."""
    from OCP.Message import Message

    messenger = Message.DefaultMessenger_s()
    printers = messenger.Printers()
    for index in range(printers.Length(), 0, -1):
        messenger.RemovePrinter(printers.Value(index))


def write(part, unit: str, path: Path) -> Path:
    """Writes `part` with write.step.unit set to `unit`."""
    from OCP.IFSelect import IFSelect_RetDone
    from OCP.Interface import Interface_Static
    from OCP.STEPControl import STEPControl_AsIs, STEPControl_Writer

    # The writer registers the write.step.* parameters, so it comes first.
    writer = STEPControl_Writer()
    if not Interface_Static.SetCVal_s("write.step.unit", unit):
        raise RuntimeError(f"Open CASCADE does not accept write.step.unit = {unit}")
    if writer.Transfer(part, STEPControl_AsIs) != IFSelect_RetDone:
        raise RuntimeError(f"Open CASCADE could not transfer {path.name} in {unit}")
    if writer.Write(str(path)) != IFSelect_RetDone:
        raise OSError(f"Open CASCADE could not write {path}")
    return path


def write_plates(out: Path) -> list[Path]:
    """Writes the plate once per length unit."""
    part = plate()
    return [write(part, unit, out / f"plate_{unit.lower()}.step") for unit in PLATE_UNITS]


def write_cone_void(out: Path) -> list[Path]:
    """Writes the part with every elementary surface in mm and in inches."""
    part = cone_void()
    return [write(part, unit, out / f"cone_void_{unit.lower()}.step") for unit in ("MM", "INCH")]


def main() -> int:
    if len(sys.argv) != 2:
        print(__doc__.split("\n\n")[2], file=sys.stderr)
        return 2
    out = Path(sys.argv[1])
    out.mkdir(parents=True, exist_ok=True)
    quiet()
    for path in write_plates(out) + write_cone_void(out) + write_assemblies(out / "assemblies"):
        print(path)
    return 0


if __name__ == "__main__":
    sys.exit(main())
