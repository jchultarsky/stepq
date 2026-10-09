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


def quiet() -> None:
    """Stops Open CASCADE printing transfer statistics for every file."""
    from OCP.Message import Message

    messenger = Message.DefaultMessenger_s()
    printers = messenger.Printers()
    for index in range(printers.Length(), 0, -1):
        messenger.RemovePrinter(printers.Value(index))


def write_plates(out: Path) -> list[Path]:
    """Writes the plate once per length unit."""
    from OCP.IFSelect import IFSelect_RetDone
    from OCP.Interface import Interface_Static
    from OCP.STEPControl import STEPControl_AsIs, STEPControl_Writer

    part = plate()
    written = []
    for unit in PLATE_UNITS:
        # The writer registers the write.step.* parameters, so it comes first.
        writer = STEPControl_Writer()
        if not Interface_Static.SetCVal_s("write.step.unit", unit):
            raise RuntimeError(f"Open CASCADE does not accept write.step.unit = {unit}")
        if writer.Transfer(part, STEPControl_AsIs) != IFSelect_RetDone:
            raise RuntimeError(f"Open CASCADE could not transfer the plate in {unit}")
        path = out / f"plate_{unit.lower()}.step"
        if writer.Write(str(path)) != IFSelect_RetDone:
            raise OSError(f"Open CASCADE could not write {path}")
        written.append(path)
    return written


def main() -> int:
    if len(sys.argv) != 2:
        print(__doc__.split("\n\n")[2], file=sys.stderr)
        return 2
    out = Path(sys.argv[1])
    out.mkdir(parents=True, exist_ok=True)
    quiet()
    for path in write_plates(out):
        print(path)
    return 0


if __name__ == "__main__":
    sys.exit(main())
