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
built with the serde feature (examples/CHECK.rs, one JSON object per line),
Open CASCADE's from the function of the same name in tools/verify-occt.py.
Both are joined on Open CASCADE's entity number, which is the instance's
1-based position in the file, and the entity types must agree.

    tools/verify-read.py CHECK [PATH...]

PATH is a STEP file or a directory searched for them; directories skip files
over 16 MB unless STEPQ_LARGE_FIXTURES=1. Default: tests/fixtures.

Checks:

  units   Every representation context that assigns units: its length,
          plane angle and solid angle factors, and its smallest length
          uncertainty in millimetres, to a relative 1e-12. Where Open CASCADE
          finds no unit of a kind, stepq must report it missing; any other
          refusal by stepq fails.

Exit status: 0 if every comparison holds, 1 if one fails, 2 if a file cannot
be read by either side.
"""

from __future__ import annotations

import importlib.util
import json
import math
import os
import subprocess
import sys
from dataclasses import dataclass
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


def step_files(paths: list[Path]) -> list[Path]:
    """The STEP files named, and those under the directories named."""
    large = os.environ.get("STEPQ_LARGE_FIXTURES", "") not in ("", "0")
    files, skipped = [], 0
    for path in paths:
        if path.is_dir():
            for found in sorted(path.rglob("*")):
                if found.suffix.lower() not in (".stp", ".step"):
                    continue
                if not large and found.stat().st_size > LARGE_FIXTURE_BYTES:
                    skipped += 1
                    continue
                files.append(found)
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


CHECKS = {
    # check: (the example that prints stepq's side, the comparison)
    "units": ("units", check_units),
}


def main() -> int:
    if len(sys.argv) < 2 or sys.argv[1] not in CHECKS:
        print("\n\n".join(__doc__.split("\n\n")[2:4]), file=sys.stderr)
        print(f"checks: {', '.join(CHECKS)}", file=sys.stderr)
        return 2
    name = sys.argv[1]
    example, check = CHECKS[name]
    files = step_files([Path(arg) for arg in sys.argv[2:]] or [ROOT / "tests" / "fixtures"])
    if not files:
        print("no STEP files (run tools/fetch-fixtures.sh)", file=sys.stderr)
        return 2
    subprocess.run(
        ["cargo", "build", "--release", "--quiet", "--features", "serde", "--example", example],
        cwd=ROOT,
        check=True,
    )

    occt.quiet()
    tally, failed, unreadable = Tally(), 0, 0
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

    print()
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
