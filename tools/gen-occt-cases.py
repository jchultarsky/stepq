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
    for path in write_plates(out) + write_cone_void(out):
        print(path)
    return 0


if __name__ == "__main__":
    sys.exit(main())
