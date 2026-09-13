# Roadmap

Ordered by leverage per line of code. Nothing here needs a geometry
kernel; that is the constraint that keeps the project finishable.

0.1.0 contains the parser, structure and split milestones below; 0.2.0
adds the inspect milestone. Open items carry over.

## Parser

- [x] Part 21 lexer: strings (`''`, `\X2\`, `\S\`), `/* */` comments,
      `#id`, `$`, `*`, enums, binaries, typed values, complex instances.
      Not line-oriented; must handle ~1 MB single lines.
- [x] Parser into an untyped entity graph with forward and back indices.
- [x] Writer that reproduces unchanged entities verbatim and renumbers
      densely. Must not touch `#` inside string literals.
- [x] EXPRESS schema reader, used only for attribute counts and aggregate
      bounds. Schemas are read at run time, not shipped (ISO copyright).
      Tested against AP203 ed1/ed2, AP214 ed3 and AP242 ed4.
- [x] AP242 editions 1–3: ISO only hosts the current edition, so files
      of every AP242 edition are checked against edition 4. Accepted; an
      older schema is used only if one becomes available.
- [x] `stepq info`
- [x] Fuzz targets for the lexer and parser.

## Structure

- [x] Product tree: `product` → `product_definition_formation` →
      `product_definition`, NAUO, CDSR / RRWT / IDT.
- [x] Placements expressed only through `mapped_item` /
      `representation_map`.
- [x] Trust the NAUO over the SRR direction (real files reverse it).
- [x] `stepq tree`, `stepq bom` (quantities via NAUO count and
      `quantified_assembly_component_usage`; multi-level tree, indented
      CSV, nested JSON, `--flat`).

## Split

- [x] Back-reference closure with the empirically grown rule table
      (`model::RULES`: shapes, placements, styles, PMI, assembly usages),
      every attribute position checked against the schemas.
- [x] Aggregate filtering for entities shared across parts.
- [x] Per-output `representation_context` handling (never merge): contexts
      are copied as referenced, never deduplicated.
- [x] Assembly-scope validation properties on re-root: not needed. An
      extraction carries only its root's own properties, which describe
      its own sub-tree; declared volumes match Open CASCADE in every AS1
      split output (`docs/ARCHITECTURE.md`).
- [x] `stepq split` — self-contained mode.
- [x] `stepq split --bodies` — multi-body products into one file per solid.
- [x] `--report-orphans`: print everything left outside the closure.
- [ ] Grow the rule table from the orphan reports. Persistent identifiers
      are done (orphaned `id_attribute`s on the core fixtures: 3,298 → 55).
      On every fixture under 16 MB, `split --report-orphans` leaves 43,534
      instances in no output, none of them solids: PMI presentation on NIST
      AP242 and AP203 (annotation planes, camera models, tessellated and
      polyline annotation, 1,164 `coordinates_list`), saved views on the
      Creo Open Rack files (`draughting_model`, `camera_usage`,
      `constructive_geometry_representation`, their points and
      directions), and moon buggy's 186 `personal_address`es.
- [x] `tools/verify-occt.py`: Σ volume(outputs) == volume(input subtree),
      equal solid counts, no lost names or colours. Already run in CI on
      every fixture rewritten by stepq (`tools/verify-rewrite.sh`).

## Inspect (0.2.0)

- [x] `stepq lint`: dangling refs, duplicate ids, attribute counts,
      empty `SET[1:?]`, shared contexts across an RRWT, missing
      `application_protocol_definition` / `product_related_product_category`.
- [x] `stepq query` / `stepq refs`: entity search and reverse lookup.
- [x] `stepq props`: user-defined attributes, validation properties,
      persistent IDs.
- [x] `stepq diff`: structural diff of two files.

## Later

- [x] `stepq pmi`: semantic GD&T to JSON.
- [x] `stepq strip` / `--anonymize`.
- [x] `stepq split --master`: CAx-IF external references instead of copies.
- [x] `stepq assemble`: the inverse of `split --master`, from the CAx-IF
      external references themselves rather than a separate manifest.
- [x] Part 21 edition 3 anchors/references.

## Not planned

Tessellation, mass properties, bounding boxes, unit conversion, healing,
any conversion to or from another format. Use a kernel.
