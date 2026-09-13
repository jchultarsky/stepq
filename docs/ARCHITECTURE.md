# Architecture

This document records the design constraints `stepq` is built on and the
evidence behind them. Read it before changing `src/model` or `src/p21`.

## One-paragraph summary

A STEP file is a graph of entity instances. Everything `stepq` does is a
graph operation: pick a root, compute a closure, filter shared
aggregates, renumber, write. No entity that describes geometry is ever
interpreted; it is copied verbatim. The single hardest problem is that
the closure cannot be computed by following references forward.

## The back-reference problem

Forward closure from a `product_definition` reaches exactly six
entities — `product_definition`, `product_definition_formation`,
`product`, `product_context`, `product_definition_context`,
`application_context` — and **zero geometry**. This was measured on the
STEP Tools AS1 assembly, and it holds for every AP.

The reason is that the schema is written "definition points at
definee": `product_definition_shape.definition → product_definition`,
`shape_definition_representation.definition → product_definition_shape`,
`styled_item.item → representation_item`,
`geometric_tolerance.toleranced_shape_aspect → shape_aspect`, and so on.
On a NIST AP242 part with PMI, 34% of all entities (1,491 of 4,350, in
84 types) are reachable only backwards.

Consequences:

1. `model::Graph` maintains a back index from day one. It is not an
   optimisation.
2. Closure is a fixpoint over a **rule table**: "if an entity of type T
   references anything currently retained, pull T in and forward-close
   it". The table is grown empirically from real files, and the tool
   must be able to report what it left behind (`--report-orphans`).
3. Type-level analysis of the EXPRESS schema is a starting point, not
   the answer. `styled_item` and `mapped_item` are subtypes of
   `representation_item`, so a schema-derived rule set misclassifies
   them.

## Failure is silent

Deleting one `shape_representation_relationship` from an otherwise
valid part file produces a file that Open CASCADE reads with zero
warnings, reporting the correct product name and **zero solids,
volume 0**. Forgetting `context_dependent_shape_representation` does the
same to a sub-assembly. NIST's STEP File Analyzer does not check this;
OCCT does not check this; nothing free checks this.

Therefore the test oracle is a *geometric invariant*, not a conformance
checker: read the input and every output back through OCCT and assert
`Σ volume(outputs) == volume(input subtree)` to 1e-9 relative, with
equal solid counts and equal name/colour sets. `tools/verify-occt.py`
implements this. It is a dev-time tool; OCCT is never a runtime
dependency.

Two measured caveats, both about OCCT rather than stepq
(`tools/verify-split.py` encodes them):

* A volume integrated at a different placement is not bit-identical.
  Summing placed component volumes of the NIST weldment differs from the
  assembly by 2e-7 relative, while every one of its 33 component shapes
  matches its own file exactly and placement determinants are 1 to
  2e-16. So components are compared unplaced at 1e-9, solid counts
  exactly, and only the placed volume sum at 1e-6.
* OCCT's default volume quadrature is not stable to 1e-9 on curved
  geometry (6.9e-7 on moon_buggy); use the adaptive integration with an
  error bound. And in a file with several top-level product definitions
  and no assembly structure, OCCT does not transfer every definition's
  shape (NIST FTC-09), so outputs there are bounded by the input rather
  than summed.

## Entities stay untyped

AP242 ed4 declares 2,407 entity types, 248 of them with multiple
supertypes. Seventeen real AP242 files use 228 types in total. Generating
a struct per type buys compile time, API churn on every schema edition,
and nothing the graph operations need.

Instances are stored as `(type name(s), attribute token list, original
byte range)`. The EXPRESS schema is loaded as data and used for exactly
two things: attribute-count validation and aggregate cardinality
(`SET[1:?]` must not be emptied by filtering).

## Verbatim output

Unchanged entities are written from their original source text. Numbers
are never re-parsed and re-printed; strings are never re-escaped. Only
`#id`s change, and the renumberer must tokenise strings first —
`PROPERTY_DEFINITION(...,'volume of #602',...)` is a real line in a real
sample file.

## Assembly structure

Two parallel trees: products (`product` → `product_definition_formation`
→ `product_definition`, linked by `next_assembly_usage_occurrence`) and
shapes (`shape_representation` linked by
`context_dependent_shape_representation` →
`(shape_representation_relationship, representation_relationship_with_transformation)`
→ `item_defined_transformation` → two `axis2_placement_3d`). A second
placement mechanism, `mapped_item` / `representation_map`, also occurs.

* `transform_item_1` is the child frame, `transform_item_2` the parent
  frame. The transform is `M(item_2) · M(item_1)⁻¹`; every exporter we
  have seen writes `item_1` as the identity.
* Real files reverse `rep_1`/`rep_2`. OCCT carries a
  `CheckSRRReversesNAUO` workaround dated 1998. **The NAUO wins.**
* In AP242, NAUO `related`/`relating` became a SELECT. Part 21 does not
  tag entity-valued SELECTs, so the record is byte-identical to AP214;
  dispatch on the target's type, not the declaring attribute.

## Extracting a sub-assembly

The transform that places sub-assembly B inside A lives on the NAUO
that is a property of the *usage*, not of B. When B becomes a root, that
NAUO is simply not retained and the transform disappears with it. No
matrix arithmetic. B comes out in whatever frame its author modelled it
in — which is not necessarily the origin. Do not promise otherwise.

## Things that must be filtered, not copied or dropped

Several entities aggregate items from many parts in one list:
`presentation_layer_assignment`, `product_related_product_category`,
`mechanical_design_geometric_presentation_representation`,
`draughting_model`, every `applied_*_assignment`, `invisibility`. Copied
wholesale they dangle; dropped they lose colours or categories. The
list is rewritten to the retained subset, and the entity is dropped only
if the subset is empty.

## Things that must not be merged

`representation_relationship_with_transformation` has a WHERE rule:
`rep_1.context_of_items :<>: rep_2.context_of_items`. Parent and child
must not share a `geometric_representation_context` even when the two
are byte-identical. OCCT reads a merged file without complaint; a
conformance checker fails it. Never deduplicate contexts.

## Multi-body split

A product whose shape representation holds several `manifold_solid_brep`
items is split by synthesising, per body: `product`,
`product_definition_formation`, `product_definition`,
`product_definition_shape`, an `advanced_brep_shape_representation`
holding one placement and the existing solid, and a
`shape_definition_representation`. Fourteen entities, zero changes to
geometry. Disconnected lumps inside a single (schema-illegal) shell are
found by union-find over shared `edge_curve` references — connectivity
only, no coordinates. A malformed `brep_with_voids` whose outer shell is
disconnected needs point-in-solid to assign voids; refuse it.

## Validation properties

Geometric validation properties (volume, area, centroid) cannot be
recomputed without a kernel, so an extraction must never carry one that
describes something else. It does not: the rule table takes only the
properties of each extracted definition and its shape aspects, and an
assembly's own properties describe its own sub-tree — exactly what its
extraction holds. Nothing needs dropping on re-root.

Measured on the AS1 splits from two exporters (CADDS via Theorem, and
Unigraphics): in all 18 output files, the root definition's declared
volume matches the volume Open CASCADE computes for that file to the
exporter's precision (for example 108452.2 declared, 108453 computed for
the L-bracket sub-assembly). `stepq props --kind validation` reads the
declared values.

## Non-goals, and why

Tessellation, mass properties, bounding boxes, healing and unit
conversion all require evaluating curves and surfaces. Open CASCADE's
STEP translator is ~300k lines on top of ~90k lines of healing and
~80k of intersection/projection. That is not a module of this project;
it is a different project.
