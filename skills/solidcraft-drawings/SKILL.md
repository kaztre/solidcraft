---
name: solidcraft-drawings
description: Use when making or changing a 2D engineering drawing of a SolidCraft design through its MCP server (sheets, views, dimensions, notes, title block, Check Drawing, PDF, SVG or PNG export), or when asked for a drawing, print or PDF of a part.
---

# SolidCraft drawings

## Overview
A drawing a machinist can make the part from, without the model and without asking. Every size is a dimension on a view; text is only what no dimension can say. Plan the sheet as numbers, build it in batches, then look at it and fix it until nothing overlaps. Command shapes, drafting rules, a recipe for turned parts, a worked example and traps: `reference.md`. Read it first.

## Before the first call
1. `skill_reference {"skill":"solidcraft-drawings"}`.
2. `drawing.list {}`: improve the user's drawing rather than adding one. One part per sheet.
3. `body_topology` with `faces: "cylinders"` for each body drawn: its diameters, lengths and the points that pick them.
4. Tolerances, fits, finishes and part numbers come from the user or the project's files (`project_search`). Never invent one, and never write "provisional": ask, or leave it out and say so.

## Build
1. **Sheet:** `drawing.sheet.add` with `size` and `orientation` (a new sheet copies the last one's paper), then `drawing.sheet.show` so the user sees it.
2. **Views:** the fewest that show every feature. Main view as the part is made (a turned part's axis horizontal), an end or section view only for what the main view hides, a small isometric in a free corner. One scale that fills about half the sheet's width.
3. **Title block:** `drawing.sheet.set` with `documents`; title and number in the block, never as a heading on the sheet.
4. **Dimensions:** each size once, on the view that shows it true. Diameters with the `cylinder` kind (reads Ø), lengths from one datum face, the overall length outside them. `decimals` to suit the tolerance.
5. **Place:** every dimension line outside its view's `boundary_mm` (`drawing.dimension.edit` with `line_through_mm`): first row 10 mm out, each further row 10 mm more, a diameter beside the feature it sizes.
6. **Callouts:** a thread, chamfer or finish is a note with a leader: `anchor` `{edge}` at an edge's `mid` from `model.edges`. General notes are one numbered note near the title block.

## Verify, then fix
- `drawing.check {"sheet": …}`, the sheet you changed, not every sheet each time: nothing lost, failed or off the sheet. One `drawing.check {}` of the whole drawing at the end.
- `drawing.scene` lists `objects` with `bounds_mm`: no note or dimension text may overlap another or a view.
- `drawing.sheet.show`, then `screenshot {"source":"window"}`. Read it as the machinist: can every feature be made and measured? Fix and look again.

## Red flags

| Thought | Do instead |
|---|---|
| "List the sizes as notes beside the view" | A size is a dimension on the view. A list of typed sizes is not a drawing. |
| "Add a heading, a status line, a view caption" | The title block says it. Leave the sheet to the part. |
| "`offset_mm` puts it outside" | It counts from the midpoint of what is measured; use `line_through_mm`. |
| "The reply says ok" | Look at the sheet. Overlaps and crowding show only there. |
| "Views are placed, the sheet is done" | Done is every feature dimensioned (`coverage`). Say what isn't. |
| "This tolerance is typical" | Ask. A guessed tolerance scraps parts. |
