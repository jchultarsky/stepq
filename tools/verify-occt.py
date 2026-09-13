#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.10,<3.14"
# dependencies = ["cadquery-ocp==8.0.1.0.0"]
# ///
"""Geometric oracle for stepq: read STEP files through Open CASCADE and compare.

stepq never evaluates geometry, and a structurally broken STEP file can read
"successfully" with zero solids and no warning (docs/ARCHITECTURE.md,
"Failure is silent"). So stepq's outputs are checked here, at development
time only: read the input and every output through Open CASCADE and require
that the outputs together have the same number of solids, the same total
volume (to a relative 1e-9), and every product name and colour of the input.

    tools/verify-occt.py summary FILE...
    tools/verify-occt.py compare INPUT OUTPUT...

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
        # Adaptive integration with an error bound: the default quadrature
        # differs by up to ~1e-6 relative between runs of the same geometry.
        properties = GProp_GProps()
        BRepGProp.VolumeProperties_s(shape, properties, VOLUME_EPS)
        return properties.Mass()

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
    args = parser.parse_args()

    try:
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
