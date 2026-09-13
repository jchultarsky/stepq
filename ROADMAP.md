# Roadmap

Ordered by leverage per line of code. Nothing here needs a geometry
kernel; that is the constraint that keeps the project finishable.

## 0.1 — the parser

- [x] Part 21 lexer: strings (`''`, `\X2\`, `\S\`), `/* */` comments,
      `#id`, `$`, `*`, enums, binaries, typed values, complex instances.
      Not line-oriented; must handle ~1 MB single lines.
- [x] Parser into an untyped entity graph with forward and back indices.
- [x] Writer that reproduces unchanged entities verbatim and renumbers
      densely. Must not touch `#` inside string literals.
- [x] EXPRESS schema reader, used only for attribute counts and aggregate
      bounds. Schemas are read at run time, not shipped (ISO copyright).
      Tested against AP203 ed1/ed2, AP214 ed3 and AP242 ed4.
- [ ] Find a source for the AP242 ed1–ed3 long-form schemas; ISO only
      hosts the current edition.
- [x] `stepq info`
- [x] Fuzz targets for the lexer and parser.

## 0.2 — structure

- [x] Product tree: `product` → `product_definition_formation` →
      `product_definition`, NAUO, CDSR / RRWT / IDT.
- [x] Placements expressed only through `mapped_item` /
      `representation_map`.
- [x] Trust the NAUO over the SRR direction (real files reverse it).
- [x] `stepq tree`, `stepq bom` (quantities via NAUO count and
      `quantified_assembly_component_usage`).

## 0.3 — split

- [ ] Back-reference closure with the empirically grown rule table
      (`context_dependent_shape_representation`, `styled_item`,
      `presentation_layer_assignment`, PMI graph, validation properties…).
- [ ] Aggregate filtering for entities shared across parts.
- [ ] Per-output `representation_context` handling (never merge).
- [ ] Drop assembly-scope validation properties on re-root.
- [ ] `stepq split` — self-contained mode.
- [ ] `stepq split --bodies` — multi-body products into one file per solid.
- [ ] `--report-orphans`: print everything left outside the closure.
- [ ] `tools/verify-occt.py`: Σ volume(outputs) == volume(input subtree).

## 0.4 — inspect

- [ ] `stepq lint`: dangling refs, duplicate ids, attribute counts,
      empty `SET[1:?]`, shared contexts across an RRWT, missing
      `application_protocol_definition` / `product_related_product_category`.
- [ ] `stepq query` / `stepq refs`: entity search and reverse lookup.
- [ ] `stepq props`: user-defined attributes, validation properties,
      persistent IDs.
- [ ] `stepq diff`: structural diff of two files.

## Later

- [ ] `stepq pmi`: semantic GD&T to JSON.
- [ ] `stepq strip` / `--anonymize`.
- [ ] `stepq split --master`: CAx-IF external references instead of copies.
- [ ] `stepq assemble`: the inverse of split, from a manifest.
- [ ] Part 21 edition 3 anchors/references.

## Not planned

Tessellation, mass properties, bounding boxes, unit conversion, healing,
any conversion to or from another format. Use a kernel.
