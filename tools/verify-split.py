#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.10,<3.14"
# dependencies = ["cadquery-ocp==8.0.1.0.0"]
# ///
"""Split fixtures with `stepq split` and check the outputs through Open CASCADE.

The geometric half of the split invariant (docs/ARCHITECTURE.md, "Failure is
silent"). For every fixture:

1. the files written for the top-level definitions together reproduce the
   input: the same solids, total volume, names and colours (relative 1e-9);
2. every part shape in a part file is one of the input's part shapes: the
   same solid count and unplaced volume (relative 1e-9), matched by value
   because product names are not unique in real files;
3. every assembly file holds exactly as many solids as its components, once
   per usage, and their volume to within ASSEMBLY_REL_TOL. Names and colours
   are held by check 1; shapes inside a representation are named differently
   by Open CASCADE depending on context. That tolerance is not about stepq: integrating curved
   surfaces at a different placement gives slightly different volumes
   (2e-7 relative on the NIST weldment, whose every component matches its own
   file exactly), and check 2 already holds each component to 1e-9.

    tools/verify-split.py [FIXTURE...]    # default: every fixture up to 16 MB

Exit status 0 if every check holds, 1 otherwise.
"""

from __future__ import annotations

import importlib.util
import json
import math
import os
import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent
_spec = importlib.util.spec_from_file_location("verify_occt", HERE / "verify-occt.py")
occt = importlib.util.module_from_spec(_spec)
# dataclasses look the module up in sys.modules while it executes.
sys.modules["verify_occt"] = occt
_spec.loader.exec_module(occt)

STEPQ = ROOT / "target" / "release" / "stepq"
ASSEMBLY_REL_TOL = 1e-6
# Without arguments, larger fixtures are skipped unless STEPQ_LARGE_FIXTURES=1:
# Open CASCADE needs minutes for each of the Open Compute assemblies.
LARGE_FIXTURE_BYTES = 16 * 1024 * 1024


def stepq(*args: str) -> str:
    return subprocess.run(
        [str(STEPQ), *args], check=True, capture_output=True, text=True
    ).stdout


def check(fixture: Path, scratch: Path) -> list[str]:
    out = scratch / fixture.stem
    manifest = json.loads(stepq("split", str(fixture), "--out", str(out), "--format", "json"))
    tree = json.loads(stepq("tree", str(fixture), "--format", "json"))
    file_of = {
        entry["definition"]["instance"]: out / entry["file"] for entry in manifest["files"]
    }
    definitions, usages = tree["definitions"], tree["usages"]
    summaries: dict[Path, occt.Summary] = {}

    def summary(path: Path) -> occt.Summary:
        if path not in summaries:
            summaries[path] = occt.read(path)
        return summaries[path]

    def output(definition: int) -> Path:
        return file_of[definitions[definition]["instance"]]

    problems = []
    source = occt.read(fixture)
    tops = [summary(output(root)) for root in tree["roots"]]
    if usages or len(tops) == 1:
        problems += [f"top-level outputs vs input: {p}" for p in occt.compare(source, tops)]
    else:
        # Several top-level definitions and no assembly structure: their
        # outputs may share geometry (PMI on one definition can refer to
        # datums on another, as in NIST FTC-09), so they are not summed.
        # No output may hold more than the input, and together they must
        # keep every name and colour.
        for top, root in zip(tops, tree["roots"]):
            if top.solids > source.solids or top.volume > source.volume * (1 + occt.REL_TOL):
                problems.append(
                    f"{output(root).name} holds more than the input: {top.line()} vs {source.line()}"
                )
        names = set().union(*(top.names for top in tops))
        colours = set().union(*(top.colours for top in tops))
        if missing := sorted(source.names - names):
            problems.append(f"names missing from all outputs: {missing}")
        if missing := sorted(source.colours - colours):
            problems.append(f"colours missing from all outputs: {missing}")

    parents = {usage["parent"] for usage in usages}

    # Open CASCADE does not transfer every definition's shape from a file with
    # several top-level definitions and no assembly structure (NIST FTC-09),
    # so there the input's part shapes cannot be listed; check 1 applies.
    flat_multi_root = not usages and len(tops) > 1
    unmatched = list(source.prototypes)
    for index in range(len(definitions)):
        if index in parents or flat_multi_root:
            continue
        part = summary(output(index))
        # Compare unplaced shapes with unplaced shapes: a part's solid may be
        # positioned inside its representation, and volumes integrated at a
        # different placement differ slightly.
        for prototype in part.prototypes:
            match = next(
                (
                    candidate
                    for candidate in unmatched
                    if candidate[0] == prototype[0]
                    and math.isclose(candidate[1], prototype[1], rel_tol=occt.REL_TOL)
                ),
                None,
            )
            if match is None:
                problems.append(
                    f"{output(index).name}: part shape with {prototype[0]} solids, "
                    f"volume {prototype[1]!r} matches no part shape of the input"
                )
            else:
                unmatched.remove(match)

    for index in sorted(parents):
        whole = summary(output(index))
        components = []
        for usage in usages:
            if usage["parent"] == index:
                copies = max(1, round(usage.get("quantity") or 1))
                components += [summary(output(usage["child"]))] * copies
        label = output(index).name
        solids = sum(component.solids for component in components)
        volume = sum(component.volume for component in components)
        if solids != whole.solids:
            problems.append(f"{label}: {whole.solids} solids, its components have {solids}")
        if not math.isclose(volume, whole.volume, rel_tol=ASSEMBLY_REL_TOL):
            problems.append(
                f"{label}: volume {whole.volume!r}, its components {volume!r} "
                f"(relative difference {abs(volume - whole.volume) / whole.volume:.3e})"
            )
    return problems


def main() -> int:
    fixtures = [Path(arg) for arg in sys.argv[1:]]
    if not fixtures:
        found = sorted(
            p
            for p in (ROOT / "tests" / "fixtures").rglob("*")
            if p.suffix.lower() in (".stp", ".step")
        )
        large = os.environ.get("STEPQ_LARGE_FIXTURES", "") not in ("", "0")
        fixtures = [p for p in found if large or p.stat().st_size <= LARGE_FIXTURE_BYTES]
        if len(fixtures) < len(found):
            print(
                f"skipping {len(found) - len(fixtures)} fixtures over 16 MB "
                "(set STEPQ_LARGE_FIXTURES=1 to include them)",
                file=sys.stderr,
            )
    if not fixtures:
        print("no fixtures (run tools/fetch-fixtures.sh)", file=sys.stderr)
        return 2
    subprocess.run(
        ["cargo", "build", "--release", "--quiet", "--bin", "stepq"],
        cwd=ROOT,
        check=True,
    )

    failed = 0
    with tempfile.TemporaryDirectory(prefix="stepq-split-") as scratch:
        for index, fixture in enumerate(fixtures):
            try:
                problems = check(fixture, Path(scratch) / str(index))
            except (OSError, subprocess.CalledProcessError) as error:
                detail = getattr(error, "stderr", "") or error
                problems = [f"could not split or read: {str(detail).strip()}"]
            name = fixture.relative_to(ROOT) if fixture.is_relative_to(ROOT) else fixture
            if problems:
                failed += 1
                print(f"FAIL {name}", flush=True)
                for problem in problems:
                    print(f"  {problem}", flush=True)
            else:
                print(f"OK   {name}", flush=True)

    print()
    if failed:
        print(f"{failed} of {len(fixtures)} fixtures failed", file=sys.stderr)
        return 1
    print(f"all {len(fixtures)} split fixtures match through Open CASCADE")
    return 0


if __name__ == "__main__":
    sys.exit(main())
