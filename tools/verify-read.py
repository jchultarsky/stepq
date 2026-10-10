#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.10,<3.14"
# dependencies = ["cadquery-ocp==8.0.1.0.0"]
# ///
"""Check what stepq reads from STEP files against what Open CASCADE reads.

stepq reads what a file states and never computes what it does not; reading
it wrong is as silent as dropping an entity (docs/ARCHITECTURE.md, "Reading
stated geometry"). So every reader is held to Open CASCADE's reading of the
same files, at development time only. stepq's side comes from an example
built with the serde feature (examples/units.rs, examples/geometry.rs; one
JSON object per line), Open CASCADE's from the function of the same name as
the check in tools/verify-occt.py. Unit contexts are joined on Open
CASCADE's entity number, which is the instance's 1-based position in the
file, and the entity types must agree; faces and edges, which Open CASCADE
does not bind to the entities it reads them from, are compared as multisets
per file.

    tools/verify-read.py CHECK [PATH...]

PATH is a STEP file or a directory searched for them, following symbolic
links; directories skip files over 16 MB unless STEPQ_LARGE_FIXTURES=1, and a
directory that holds no STEP file at all is an error. Default: tests/fixtures.

Checks:

  units     Every representation context that assigns units: its length,
            plane angle and solid angle factors, and its smallest length
            uncertainty in millimetres, to a relative 1e-12. Where Open
            CASCADE finds no unit of a kind, stepq must report it missing;
            any other refusal by stepq fails.

  surfaces  The surface of every face a shape representation reaches,
            against Open CASCADE's part prototypes (unplaced, each part once,
            BRepAdaptor_Surface per face), as multisets per file: the same
            number of planes, cylinders, cones, spheres, tori and others, and
            each elementary surface equal to one of Open CASCADE's, one to
            one, by a key that x does not enter: a plane's signed normal and
            offset, a cylinder's axis line (unsigned) and radius, a cone's
            apex, signed axis and semi-angle, a sphere's centre and radius, a
            torus's centre, unsigned axis and radii. Any refusal by stepq
            fails. Open CASCADE clamps or drops cones whose semi-angle stepq
            refuses, so those are covered by unit tests only.

  curves    The 3D curve of every edge of those faces (a surface curve's
            curve_3d), against BRepAdaptor_Curve: each line (signed
            direction, and where it runs) and circle (centre, signed normal,
            radius) equal to one of Open CASCADE's, one to one. Open CASCADE
            heals while it reads: it adds a seam to a periodic face that the
            file bounds only by closed loops and splits the closed edges
            there, so it may hold more; a line or circle it holds beyond
            stepq's must be such a seam or a piece of an edge stepq reads, or
            the file fails. Edges of no face (a wireframe's) are left out on
            both sides.

Both geometry checks compare each value to 1e-9 of max(1, |value|, the size
of the stated location it is computed from), and report positions on (-1, 0,
0) with no ref_direction in AP203 edition 1 files, whose x that schema leaves
undefined (stepq reads ref_direction_stated = false).

Exit status: 0 if every comparison holds, 1 if one fails, 2 if a file cannot
be read by either side or a directory named holds no STEP file.
"""

from __future__ import annotations

import importlib.util
import json
import math
import os
import re
import subprocess
import sys
from dataclasses import dataclass, field
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent
_spec = importlib.util.spec_from_file_location("verify_occt", HERE / "verify-occt.py")
occt = importlib.util.module_from_spec(_spec)
# dataclasses look the module up in sys.modules while it executes.
sys.modules["verify_occt"] = occt
_spec.loader.exec_module(occt)

REL_TOL = 1e-12
LARGE_FIXTURE_BYTES = 16 * 1024 * 1024


class Unreadable(Exception):
    """One side could not read a file at all."""


class NoFiles(Exception):
    """A directory named holds no STEP file."""


def files_under(directory: Path) -> list[Path]:
    """Every STEP file under `directory`, sorted.

    Symlinked directories are followed, as the Rust fixture tests follow
    them (Path.rglob does not), and each real directory is read once.
    """
    found, seen = [], set()
    for root, dirs, names in os.walk(directory, followlinks=True):
        real = os.path.realpath(root)
        if real in seen:
            dirs[:] = []
            continue
        seen.add(real)
        found.extend(Path(root) / name for name in names
                     if Path(name).suffix.lower() in (".stp", ".step"))
    return sorted(found)


def step_files(paths: list[Path]) -> list[Path]:
    """The STEP files named, and those under the directories named.

    Each directory must hold at least one, before the size limit, so that a
    set that was never fetched cannot pass unseen next to one that was.
    """
    large = os.environ.get("STEPQ_LARGE_FIXTURES", "") not in ("", "0")
    files, skipped = [], 0
    for path in paths:
        if path.is_dir():
            found = files_under(path)
            if not found:
                raise NoFiles(f"no STEP files under {path} (run tools/fetch-fixtures.sh)")
            for file in found:
                if not large and file.stat().st_size > LARGE_FIXTURE_BYTES:
                    skipped += 1
                    continue
                files.append(file)
        else:
            files.append(path)
    if skipped:
        print(
            f"skipping {skipped} files over 16 MB (set STEPQ_LARGE_FIXTURES=1 to include them)",
            file=sys.stderr,
        )
    return files


def stepq_lines(example: str, path: Path) -> list[dict]:
    """The JSON lines an example prints for one file."""
    binary = ROOT / "target" / "release" / "examples" / example
    run = subprocess.run([str(binary), str(path)], capture_output=True, text=True)
    if run.returncode != 0:
        raise Unreadable(f"stepq: {run.stderr.strip()}")
    return [json.loads(line) for line in run.stdout.splitlines()]


@dataclass
class Tally:
    """What a check compared, over every file."""

    files: int = 0
    items: int = 0
    agreeing: int = 0
    values: int = 0
    identical: int = 0
    close: int = 0
    absent: int = 0

    def value(self, ours: float, theirs: float) -> str | None:
        """Compares two values; returns why they differ, if they do."""
        self.values += 1
        if ours == theirs:
            self.identical += 1
            return None
        if math.isclose(ours, theirs, rel_tol=REL_TOL, abs_tol=0.0):
            self.close += 1
            return None
        return f"stepq {ours!r}, Open CASCADE {theirs!r}"


def same_context_type(occt_class: str, stepq_entity: str) -> bool:
    """True if Open CASCADE's class and stepq's partial entities name the
    same kind of unit context."""
    parts = set(stepq_entity.split("+"))
    return (
        "GLOBAL_UNIT_ASSIGNED_CONTEXT" in parts
        and occt_class.startswith("StepGeom_") == ("GEOMETRIC_REPRESENTATION_CONTEXT" in parts)
        and ("Uncertainty" in occt_class) == ("GLOBAL_UNCERTAINTY_ASSIGNED_CONTEXT" in parts)
    )


def check_units(path: Path, tally: Tally) -> list[str]:
    """Compares every unit context of one file; returns the differences."""
    try:
        theirs = {context.rank: context for context in occt.units(path)}
    except OSError as error:
        raise Unreadable(str(error)) from error
    ours = {line["rank"]: line for line in stepq_lines("units", path)}
    problems = []
    for rank in sorted(set(theirs) | set(ours)):
        line, context = ours.get(rank), theirs.get(rank)
        tally.items += 1
        if context is None:
            problems.append(f"#{line['context']}: stepq lists a unit context Open CASCADE does not")
            continue
        if line is None:
            problems.append(f"entity {rank} ({context.entity}): stepq lists no unit context")
            continue
        where = f"#{line['context']} (entity {rank})"
        if not same_context_type(context.entity, line["entity"]):
            problems.append(f"{where}: {line['entity']} is not {context.entity}")
            continue
        if "error" in line:
            problems.append(f"{where}: stepq refuses the context: {line['error']}")
            continue
        units = line["units"]
        before = len(problems)
        for kind, value in (
            ("length", context.length),
            ("plane_angle", context.plane_angle),
            ("solid_angle", context.solid_angle),
        ):
            read = units[kind]
            if "ok" in read and value is not None:
                if (why := tally.value(read["ok"]["factor"], value)) is not None:
                    problems.append(f"{where}: {kind} factor: {why}")
            elif "error" in read and value is None and read["error"]["kind"] == "missing":
                tally.absent += 1
            elif "ok" in read:
                problems.append(
                    f"{where}: {kind}: stepq reads {read['ok']['factor']!r}, Open CASCADE finds none"
                )
            else:
                problems.append(f"{where}: {kind}: stepq refuses ({read['error']}), "
                                f"Open CASCADE reads {value!r}")
        read = units["uncertainty"]
        if "error" in read:
            problems.append(f"{where}: stepq refuses the uncertainty: {read['error']}")
        else:
            lengths = [u["value"] for u in read["ok"] if u["unit"]["kind"] == "length"]
            value = min(lengths) if lengths else None
            if value is not None and context.uncertainty is not None:
                if (why := tally.value(value, context.uncertainty)) is not None:
                    problems.append(f"{where}: uncertainty: {why}")
            elif value is None and context.uncertainty is None:
                tally.absent += 1
            else:
                problems.append(
                    f"{where}: uncertainty: stepq {value!r}, Open CASCADE {context.uncertainty!r}"
                )
        if len(problems) == before:
            tally.agreeing += 1
    return problems


# The tolerance of the surfaces and curves checks: each value within this
# times max(1, |value|), so directions and offsets near zero compare
# absolutely and large coordinates relatively.
GEOMETRY_TOL = 1e-9

# stepq's surface kinds and Open CASCADE's names for them.
SURFACE_KINDS = {
    "plane": "plane",
    "cylindrical": "cylinder",
    "conical": "cone",
    "spherical": "sphere",
    "toroidal": "torus",
}


def _dot(a, b) -> float:
    return sum(x * y for x, y in zip(a, b))


def _unit(v) -> tuple[float, ...]:
    length = math.sqrt(_dot(v, v))
    return tuple(x / length for x in v)


def _foot(point, direction) -> tuple[float, ...]:
    """The point of the line through `point` along unit `direction` nearest
    the origin: where the line is, whatever point names it."""
    along = _dot(point, direction)
    return tuple(p - along * d for p, d in zip(point, direction))


@dataclass
class Key:
    """What a check compares of one surface or curve: values that do not
    depend on the position's x axis, nor on where along an axis the location
    sits. `unsigned` marks the three values of an axis whose sign means
    nothing (a cylinder's or torus's); every other axis is compared with its
    sign (a plane's normal, a cone's opening, a circle's normal, a line's
    direction).

    `scales` gives each value the magnitude it was computed from: a foot
    point or a plane's offset is a difference of terms as large as the
    stated location, so it is compared relative to that, not to its own
    size (a line stated 2.5 km from where it runs, as NIST STC-07 has one,
    loses nine digits of its foot point to cancellation on either side)."""

    kind: str
    values: tuple[float, ...]
    unsigned: int | None = None
    scales: tuple[float, ...] = ()

    def flipped(self) -> "Key":
        if self.unsigned is None:
            return self
        i = self.unsigned
        values = list(self.values)
        values[i:i + 3] = [-v for v in values[i:i + 3]]
        return Key(self.kind, tuple(values), self.unsigned, self.scales)

    def bucket(self) -> tuple:
        """A coarse hash of the key: equal keys almost always share one, and
        a key that misses its bucket is searched for among all the rest."""
        key = self
        if self.unsigned is not None:
            axis = self.values[self.unsigned:self.unsigned + 3]
            if max(axis, key=abs) < 0:
                key = self.flipped()
        return (self.kind, *(round(v, 5) for v in key.values))

    def deviation(self, other: "Key") -> float:
        """The largest difference, relative to max(1, |value|, its scale)."""
        scales = [max(a, b) for a, b in zip(self.scales or [1.0] * len(self.values),
                                             other.scales or [1.0] * len(other.values))]

        def worst(a: "Key") -> float:
            return max(
                (
                    abs(x - y) / max(1.0, abs(x), abs(y), scale)
                    for x, y, scale in zip(a.values, other.values, scales)
                ),
                default=0.0,
            )

        if self.kind != other.kind or len(self.values) != len(other.values):
            return math.inf
        if self.unsigned is None:
            return worst(self)
        return min(worst(self), worst(self.flipped()))


def _size(v) -> float:
    return math.sqrt(_dot(v, v))


def surface_key(kind: str, location, axis, a: float, b: float) -> Key:
    """The key of an elementary surface, in Open CASCADE's kind names."""
    size = _size(location)
    if kind == "plane":
        z = _unit(axis)
        return Key(kind, (*z, _dot(location, z)), scales=(1.0, 1.0, 1.0, size))
    if kind == "cylinder":
        z = _unit(axis)
        return Key(
            kind, (*z, *_foot(location, z), a), unsigned=0, scales=(1.0,) * 3 + (size,) * 3 + (1.0,)
        )
    if kind == "cone":
        # The apex, the opening axis and the semi-angle; where the reference
        # circle sits along the axis is free.
        z = _unit(axis)
        height = a / math.tan(b)
        apex = tuple(p - d * height for p, d in zip(location, z))
        return Key(kind, (*z, *apex, b), scales=(1.0,) * 3 + (size + height,) * 3 + (1.0,))
    if kind == "sphere":
        return Key(kind, (*location, a))
    if kind == "torus":
        return Key(kind, (*_unit(axis), *location, a, b), unsigned=0)
    return Key(kind, ())


def curve_key(kind: str, location, axis, a: float) -> Key:
    """The key of a line or circle."""
    if kind == "line":
        direction = _unit(axis)
        return Key(
            kind,
            (*_foot(location, direction), *direction),
            scales=(_size(location),) * 3 + (1.0,) * 3,
        )
    if kind == "circle":
        return Key(kind, (*location, *_unit(axis), a))
    return Key(kind, ())


def stepq_surface(surface: dict) -> Key:
    kind = SURFACE_KINDS.get(surface["kind"])
    if kind is None:
        return Key("other", ())
    position = surface["position"]
    a = surface.get("radius", surface.get("major_radius", 0.0))
    b = surface.get("semi_angle", surface.get("minor_radius", 0.0))
    return surface_key(kind, position["location"], position["z"], a, b)


def stepq_curve(curve: dict) -> Key:
    """The key of an edge's 3D curve: a surface curve's `curve_3d`."""
    if curve["kind"] == "surface_curve":
        curve = curve["curve_3d"]
    if curve["kind"] == "line":
        return curve_key("line", curve["pnt"], curve["dir"]["orientation"], 0.0)
    if curve["kind"] == "circle":
        position = curve["position"]
        return curve_key("circle", position["location"], position["z"], curve["radius"])
    return Key("other", ())


def occt_key(geometry) -> Key:
    """The key of Open CASCADE's reading of a surface or curve."""
    if geometry.kind.startswith("GeomAbs_"):
        return Key("other", ())
    if geometry.kind in ("line", "circle"):
        return curve_key(geometry.kind, geometry.location, geometry.axis, geometry.a)
    return surface_key(geometry.kind, geometry.location, geometry.axis, geometry.a, geometry.b)


@dataclass
class GeometryTally(Tally):
    """What the surfaces or curves check compared, over every file."""

    # Per kind: how many each side reads.
    kinds: dict = field(default_factory=dict)
    # The worst deviation of a matched pair.
    worst: float = 0.0
    # Positions on (-1, 0, 0) without a ref_direction in AP203 edition 1.
    defaulted_minus_x: int = 0
    # Curves only: what Open CASCADE's healing added.
    seams: int = 0
    pieces: int = 0
    other_extra: int = 0
    # Said about the file being checked, after its status line.
    notes: list = field(default_factory=list)


def match(ours: list[Key], theirs: list[Key]) -> tuple[list[Key], list[int], float]:
    """Pairs every key of `ours` with an equal key of `theirs`, one to one.

    Returns the keys of `ours` left unpaired, the indices of those of
    `theirs`, and the worst deviation of the pairs. Each key is looked for
    first among the keys that share its bucket, then among every key left of
    its kind.
    """
    buckets: dict[tuple, list[int]] = {}
    by_kind: dict[str, list[int]] = {}
    for i, key in enumerate(theirs):
        buckets.setdefault(key.bucket(), []).append(i)
        by_kind.setdefault(key.kind, []).append(i)
    used, unmatched, worst = set(), [], 0.0
    for key in ours:
        found = None
        for candidates in (buckets.get(key.bucket(), []), by_kind.get(key.kind, [])):
            best, best_deviation = None, math.inf
            for i in candidates:
                if i in used:
                    continue
                deviation = key.deviation(theirs[i])
                if deviation <= GEOMETRY_TOL and deviation < best_deviation:
                    best, best_deviation = i, deviation
                    if deviation == 0.0:
                        break
            if best is not None:
                found = best
                worst = max(worst, best_deviation)
                break
        if found is None:
            unmatched.append(key)
        else:
            used.add(found)
    return unmatched, [i for i in range(len(theirs)) if i not in used], worst


def file_schema(path: Path) -> str:
    """The first schema the header names, upper case."""
    with open(path, "rb") as file:
        head = file.read(1 << 16)
    found = re.search(rb"FILE_SCHEMA\s*\(\s*\(\s*'([^']*)'", head, re.IGNORECASE)
    return found.group(1).decode("ascii", "replace").upper() if found else ""


def defaulted_minus_x(value: dict) -> int:
    """1 if `value` is placed on an axis of exactly (-1, 0, 0) with no
    ref_direction: under AP203 edition 1 its x is undefined (stepq reads the
    later editions' default and says ref_direction_stated = false)."""
    if value.get("kind") == "surface_curve":
        value = value["curve_3d"]
    position = value.get("position")
    if position is None or position["ref_direction_stated"]:
        return 0
    return int(position["z"] == [-1.0, 0.0, 0.0])


def stepq_keys(path: Path, tally: GeometryTally, item: str) -> tuple[list[Key], list[str]]:
    """stepq's keys for every face, or every edge of a face, of one file,
    and its refusals. An edge reached from no face (a wireframe's) is left
    out: Open CASCADE reads it as an edge of no face."""
    field_name = "surface" if item == "face" else "curve"
    lines = [
        line
        for line in stepq_lines("geometry", path)
        if line["item"] == item and (item == "face" or "face" in line)
    ]
    keys, problems, minus_x = [], [], 0
    for line in lines:
        if "error" in line:
            problems.append(f"#{line['id']} (entity {line['rank']}): stepq refuses it: {line['error']}")
            continue
        value = line[field_name]
        minus_x += defaulted_minus_x(value)
        keys.append(stepq_surface(value) if item == "face" else stepq_curve(value))
    tally.items += len(lines)
    if minus_x and file_schema(path) == "CONFIG_CONTROL_DESIGN":
        tally.defaulted_minus_x += minus_x
        tally.notes.append(
            f"{minus_x} positions on (-1, 0, 0) with no ref_direction, whose x AP203 edition 1 "
            "leaves undefined, read with ref_direction_stated = false"
        )
    return keys, problems


def count_kinds(tally: GeometryTally, ours: list[Key], theirs: list[Key]) -> dict:
    """How many of each kind each side reads, added to the tally."""
    counts: dict[str, dict[str, int]] = {}
    for side, keys in (("stepq", ours), ("occt", theirs)):
        for key in keys:
            counts.setdefault(key.kind, {"stepq": 0, "occt": 0})[side] += 1
    for kind, count in counts.items():
        total = tally.kinds.setdefault(kind, {"stepq": 0, "occt": 0})
        total["stepq"] += count["stepq"]
        total["occt"] += count["occt"]
    return counts


def compare_elementary(tally: GeometryTally, ours: list[Key], theirs: list[Key], problems: list[str]):
    """Matches stepq's elementary keys with Open CASCADE's one to one;
    returns the indices of Open CASCADE's keys left over."""
    elementary = [key for key in ours if key.kind != "other"]
    indices = [i for i, key in enumerate(theirs) if key.kind != "other"]
    unmatched, left, worst = match(elementary, [theirs[i] for i in indices])
    tally.values += len(elementary)
    tally.close += len(elementary) - len(unmatched)
    tally.worst = max(tally.worst, worst)
    for key in unmatched[:5]:
        problems.append(f"stepq's {key.kind} {key.values} has no match in Open CASCADE")
    if len(unmatched) > 5:
        problems.append(f"... and {len(unmatched) - 5} more of stepq's")
    return [indices[i] for i in left]


def check_surfaces(path: Path, tally: GeometryTally) -> list[str]:
    """Compares the surface of every face of one file: the same number of
    each kind, and each elementary surface equal to one of Open CASCADE's."""
    try:
        theirs = [occt_key(geometry) for geometry in occt.surfaces(path)]
    except OSError as error:
        raise Unreadable(str(error)) from error
    ours, problems = stepq_keys(path, tally, "face")
    for kind, count in sorted(count_kinds(tally, ours, theirs).items()):
        if count["stepq"] != count["occt"]:
            problems.append(f"{kind}: stepq reads {count['stepq']}, Open CASCADE {count['occt']}")
    left = compare_elementary(tally, ours, theirs, problems)
    for i in left[:5]:
        problems.append(f"Open CASCADE's {theirs[i].kind} {theirs[i].values} has no match in stepq")
    if len(left) > 5:
        problems.append(f"... and {len(left) - 5} more of Open CASCADE's")
    tally.agreeing += not problems
    return problems


def check_curves(path: Path, tally: GeometryTally) -> list[str]:
    """Compares the 3D curve of every edge of a face of one file.

    Every line and circle stepq reads must equal one of Open CASCADE's, one
    to one. Open CASCADE heals while it reads, so it may hold more: a seam
    it adds to a periodic face the file bounds only by closed loops (any
    kind of curve), and the pieces of a closed edge it splits there (the
    same line or circle as an edge stepq reads). Any other line or circle it
    holds fails, as does reading fewer other curves than stepq.
    """
    try:
        geometries = occt.curves(path)
    except OSError as error:
        raise Unreadable(str(error)) from error
    theirs = [occt_key(geometry) for geometry in geometries]
    ours, problems = stepq_keys(path, tally, "edge")
    counts = count_kinds(tally, ours, theirs)
    seams = pieces = 0
    unexplained = []
    for i in compare_elementary(tally, ours, theirs, problems):
        if geometries[i].seam:
            seams += 1
        elif any(theirs[i].deviation(key) <= GEOMETRY_TOL for key in ours):
            pieces += 1
        else:
            unexplained.append(theirs[i])
    for key in unexplained[:5]:
        problems.append(
            f"Open CASCADE's {key.kind} {key.values} is neither an edge stepq reads, nor a "
            "seam, nor a piece of one"
        )
    if len(unexplained) > 5:
        problems.append(f"... and {len(unexplained) - 5} more of Open CASCADE's")
    other = counts.get("other", {"stepq": 0, "occt": 0})
    if other["stepq"] > other["occt"]:
        problems.append(f"other: stepq reads {other['stepq']}, Open CASCADE {other['occt']}")
    tally.seams += seams
    tally.pieces += pieces
    tally.other_extra += max(0, other["occt"] - other["stepq"])
    if seams or pieces or other["occt"] > other["stepq"]:
        tally.notes.append(
            f"Open CASCADE adds {seams} lines and circles as seams, splits closed ones into "
            f"{pieces} more pieces, and holds {other['occt'] - other['stepq']} more other curves"
        )
    tally.agreeing += not problems
    return problems


CHECKS = {
    # check: (the example that prints stepq's side, the comparison, its tally)
    "units": ("units", check_units, Tally),
    "surfaces": ("geometry", check_surfaces, GeometryTally),
    "curves": ("geometry", check_curves, GeometryTally),
}


def main() -> int:
    if len(sys.argv) < 2 or sys.argv[1] not in CHECKS:
        print("\n\n".join(__doc__.split("\n\n")[2:4]), file=sys.stderr)
        print(f"checks: {', '.join(CHECKS)}", file=sys.stderr)
        return 2
    name = sys.argv[1]
    example, check, tally_type = CHECKS[name]
    try:
        files = step_files([Path(arg) for arg in sys.argv[2:]] or [ROOT / "tests" / "fixtures"])
    except NoFiles as error:
        print(error, file=sys.stderr)
        return 2
    if not files:
        print("no STEP files (run tools/fetch-fixtures.sh)", file=sys.stderr)
        return 2
    subprocess.run(
        ["cargo", "build", "--release", "--quiet", "--features", "serde", "--example", example],
        cwd=ROOT,
        check=True,
    )

    occt.quiet()
    tally, failed, unreadable = tally_type(), 0, 0
    for path in files:
        resolved = path.resolve()
        label = resolved.relative_to(ROOT) if resolved.is_relative_to(ROOT) else path
        try:
            problems = check(path, tally)
        except Unreadable as error:
            unreadable += 1
            print(f"ERROR {label}: {error}", flush=True)
            continue
        tally.files += 1
        if problems:
            failed += 1
            print(f"FAIL {label}", flush=True)
            for problem in problems:
                print(f"  {problem}", flush=True)
        else:
            print(f"OK   {label}", flush=True)
        for note in getattr(tally, "notes", []):
            print(f"     {note}", flush=True)
        if isinstance(tally, GeometryTally):
            tally.notes.clear()

    print()
    if isinstance(tally, GeometryTally):
        what = "faces" if name == "surfaces" else "edges"
        print(
            f"{name}: {tally.agreeing} of {tally.files} files agree; stepq reads {tally.items} "
            f"{what}, and {tally.close} of its {tally.values} elementary values match Open "
            f"CASCADE's one to one (worst deviation {tally.worst:.2g}, tolerance "
            f"{GEOMETRY_TOL:g})"
        )
        for kind, count in sorted(tally.kinds.items()):
            print(f"  {kind}: stepq {count['stepq']}, Open CASCADE {count['occt']}")
        if name == "curves":
            print(
                f"  Open CASCADE's healing: {tally.seams} lines and circles added as seams, "
                f"{tally.pieces} more pieces of split closed edges, {tally.other_extra} more "
                "other curves (seams and pieces, not compared)"
            )
        if tally.defaulted_minus_x:
            print(
                f"  {tally.defaulted_minus_x} positions on (-1, 0, 0) with no ref_direction in "
                "AP203 edition 1 files, read with ref_direction_stated = false"
            )
    else:
        print(
            f"{name}: {tally.agreeing} of {tally.items} contexts agree in {tally.files} files; "
            f"of {tally.values} values compared, {tally.identical} are bit-identical and "
            f"{tally.close} within {REL_TOL:g} relative; {tally.absent} absent on both sides"
        )
    if unreadable:
        print(f"{unreadable} files could not be read", file=sys.stderr)
        return 2
    if failed:
        print(f"{failed} of {len(files)} files differ", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
