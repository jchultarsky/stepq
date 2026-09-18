# Architecture

This document records the design constraints `stepq` is built on and the
evidence behind them. Read it before changing `src/model` or `src/p21`.

It assumes familiarity with the STEP entity model: the product chain,
shape representations, `next_assembly_usage_occurrence`, styled items
and the rest. Those are not explained here. The companion book
[*Inside the STEP File*](https://jchultarsky.github.io/step-book/)
covers them from first principles, with real file excerpts, and is the
foundation this document builds on. Its source lives in the
[step-book repository](https://github.com/jchultarsky/step-book).

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
* Volume integration over a compound is not additive. The Creo Open Rack
  files write a surface model (`shell_based_surface_model` with open
  shells) next to each solid: one housing's solid integrates to 22,933.83,
  its open shells to 0, and the two together to 23,063.60. Placed in its
  assembly with 22 other solids, whose every one matches its own file to
  1e-15, the assembly integrates to 27,122.62 against components summing to
  27,096.39, a 1e-3 difference that is all Open CASCADE's. Volumes are
  therefore summed solid by solid.

## Entities stay untyped

AP242 ed4 declares 2,407 entity types, 248 of them with multiple
supertypes. Seventeen real AP242 files use 228 types in total. Generating
a struct per type buys compile time, API churn on every schema edition,
and nothing the graph operations need.

Instances are stored as `(type name(s), attribute token list, original
byte range)`. The EXPRESS schema is loaded as data, at run time, and used
only for validation: `stepq lint --schema` checks entity types, attribute
counts and aggregate lower bounds (an empty `SET[1:?]`), and the test
suite checks every attribute position in the closure rule table against
it. Nothing else reads the schema; `split` needs none.

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

A filtered list is not followed, with one exception. A `draughting_model`
lists the presentation of many PMI elements, but its saved views
(`camera_model_d3` and relatives), the shapes shown in them (`mapped_item`)
and its notes exist only as its items; nothing else refers to them. So the
rule for `draughting_model` follows items of those types, except an
`annotation_plane` whose elements semantic PMI is associated with
(`draughting_model_item_association` and relatives): that plane belongs to
the PMI's product and comes with it. Following every plane copied the
callouts of three top-level definitions into each other's files in NIST
FTC-09.

## What the rule table covers

`split --report-orphans` reports what no output holds. The table was grown
from those reports on every fixture, the CAx-IF recommended practices (PMI
Representation and Presentation, PMI Polyline Presentation, External
References, Tessellated Geometry) and the schemas, and no rule was added
without checking its position in AP242 ed. 4, AP214 and AP203 ed. 2.

On the 48 fixtures under 16 MB, instances in no output went from 43,534 to
1,559; on all 55 fixtures, including the Open Compute assemblies up to
207 MB, 2,969 remain, and no entity type grew along the way. What the
growth brought in, by family: PMI presentation (annotation planes, draughting
relationships, per-annotation validation properties, callout
relationships, display formats), saved views (AP242 cameras, Creo
presentation sets, areas and views), supplemental geometry, notes and
polyline or tessellated annotation reached through draughting models,
tolerance zones, composite tolerances, referenced standards documents,
addresses, and two subtypes the table had missed
(`directed_dimensional_location`, `default_model_geometric_view`, whose
`of_shape` is its seventh attribute). The 45 core fixtures still split to
Open CASCADE's solid counts and volumes.

None of what remains is left behind: all 2,969 instances are data nothing
refers to, directly or through other such data (`model::unreachable`) —
1,827 colours no style uses, 639 unused `dimensional_exponents`, 69
pre-defined colours, 40 product categories, dates, and the saved views of
the NIST FTC-09 draughting model no product reaches. No product can own
those, so the report lists them apart from what is left behind. Not handled on purpose: a plain
`representation_relationship` between AP203 draughting models (a rule on it
would also match other relationships), and the Creo saved views' mapped
items as back references, which would pull a parent's placements into a
child placed through `mapped_item` (the NIST moon buggy).

## Things that must not be merged

`representation_relationship_with_transformation` has a WHERE rule:
`rep_1.context_of_items :<>: rep_2.context_of_items`. Parent and child
must not share a `geometric_representation_context` even when the two
are byte-identical. OCCT reads a merged file without complaint; a
conformance checker fails it. Never deduplicate contexts.

## Multi-body split

A part whose shape holds several `manifold_solid_brep` items (Creo writes
7 and 22 into one representation in the Open Rack fixtures) is split by
`split --bodies` into one file per solid without synthesising anything:
the part is extracted with its other solids excluded
(`model::extract_excluding`). Their faces, edges and styles are never
reached, and every extracted instance that listed an excluded solid —
both shape representations that Creo writes, a layer — is pruned, so its
list keeps only the remaining solid. A reference to an excluded solid
that is not a list item cannot be pruned; the write then fails instead of
producing a dangling file. Each body file keeps the part's product, so
it reads as the same part with one body.

Not handled, and refused rather than guessed: disconnected lumps inside a
single (schema-illegal) shell, which would need union-find over shared
`edge_curve` references, and a `brep_with_voids` whose outer shell is
disconnected, which needs point-in-solid to assign its voids.

## Master files

`split --master` writes each assembly as a master file that refers to its
components' files, following the CAx-IF Recommended Practices for
External (Element) References 3.1, §6.1. Attribute order comes from the
schema, not the document: two of its own examples are wrong
(`applied_document_reference` has three attributes, and one example
reverses them).

A master keeps its own instances and, of each component, a stub: the
product, formation and definition, the shape definition, and its shape
representations reduced to their placements. It is `extract_excluding`
from the assembly with every component definition, every non-placement
item of their shape representations, and every simple
`shape_representation_relationship` to those representations excluded.
That last exclusion is not optional: exporters such as Unigraphics tie a
placement-only representation to the one holding the solid with such a
relationship, and without it the rule table pulls the geometry back in.
Per component the file gains, numbered after its highest instance name:
`document_type`, `document_file` (id = file name),
`document_representation_type('digital')`, `identification_role('external
document id and location')`, `external_source` (empty: same folder),
`applied_external_identification_assignment` (the file name, which Open
CASCADE reads first), `applied_document_reference` to the component
definition, `object_role('mandatory')` with its `role_association`, and the
'external definition' property linking the document to the stub shape.

Open CASCADE reads the top master of the AS1 assembly from three exporters
(Unigraphics, CADDS, Pro/ENGINEER), nested masters included, with exactly
the original solid count and volume, and every master file holds no
solid. It does not attach external files to components placed through
`mapped_item`s (the NIST moon buggy): its reader ties an external file to
the component definition, which mapped items bypass.

## Assembling

`assemble` is the inverse of a master. Each `applied_document_reference`
to a `document_file` names a file (the external identification's
`assigned_id`, else the document id, as Open CASCADE reads it) and the stub
definition it stands for. The stub's product, formation, definition, shape
definition, its link to a shape and its shape representations are paired
with the same instances of the file's definition with that product id;
references to them are rewritten with `p21::Replacements`, and the stub
and the reference entities are dropped. Component files are written after
the master with `Numbering::Offset`, past every name already used, so
nothing but the redirected tokens changes.

A file is merged once, however many masters name it: the AS1 nut is used
by the rod assembly and by the nut-and-bolt assembly, and assembling
nested masters one file at a time brought it back twice. Merged files are
therefore remembered by name, and a file that refers back to one being
merged is an error rather than a loop.

Measured on the AS1 assembly from four exporters (Unigraphics, CADDS,
Pro/ENGINEER, and NIST's `as1_pe`): splitting into masters and assembling
the top master gives the original products and component quantities
(`stepq diff`), and Open CASCADE reads the same solid count, volume,
names and colours as from the original file.

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
