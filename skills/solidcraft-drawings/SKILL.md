---
name: solidcraft-drawings
description: Use when making or changing a 2D engineering drawing of a SolidCraft design through its MCP server (sheets, views, dimensions, notes, title block, Check Drawing, PDF, SVG or PNG export), or when asked for a drawing, print or PDF of a part.
---

# SolidCraft drawings

## Overview
A drawing is sheets of views of the model, with dimensions, notes and a title block that read the model. Plan the sheet as numbers (paper, views, every dimension and where its line goes), build it in three `batch` calls, then check it with `drawing.check` and look at it. Command shapes, a worked example and traps: `reference.md`.

## Before the first call
1. `skill_reference {"skill":"solidcraft-drawings"}`; elsewhere read `reference.md`.
2. `drawing.list {}`: change the user's drawing rather than adding a second.
3. `inspect_design {"measure":true,"sketches":false}` for each body's box. List what needs a dimension: overall sizes, then every hole, slot and step by size and position from one datum corner.
4. The Drawings menu is a preview the user may have switched off; the commands work either way.

## Build
1. **Sheet:** `drawing.create` (ISO A3, third angle) opens the drawing; `drawing.view.standard` with `gap_mm` 35 places front, top, right and iso at a fitted scale. Add section, auxiliary or flat views where a feature hides.
2. **Title block:** `drawing.sheet.set` with `documents` (the source id `drawing.create` returned, component 0) fills the title, material and mass; `drawing.properties.set` the part number and organisation; `drawing.revision.add` the date and initials.
3. **Dimensions:** view ids are random, so read them from the first batch. `drawing.dimension.add` by model points, each on the view that shows it true and face on, `decimals` to suit the tolerance. Never on the iso view.
4. **Place:** a new dimension sits near what it measures, often inside the outline. `drawing.view.inspect` gives a view's `boundary_mm`; `drawing.dimension.edit` with `line_through_mm` moves each line outside it, the smaller ones 10 mm out, the overall ones 20 mm.
5. **Notes:** `drawing.note.add`; `at` is the note's centre, inside the border.

## Verify
- `drawing.check {}` lists what is lost, outside the sheet or empty; `coverage` names features nothing points at. Fix all but title fields the user hasn't given.
- Look: `screenshot {"source":"window"}` while the sheet is on screen, or tell the user to export a PNG.
- Export only when asked: `drawing.export` with `all_sheets` and `pdf` writes every sheet, printed at actual size.

## Red flags

| Thought | Do instead |
|---|---|
| "Model Dimensions brings every size in" | It brings sketch dimensions and extrude distances only; a box, cylinder or hole needs `drawing.dimension.add`. |
| "`offset_mm` puts it outside" | It counts from the midpoint of the measured points; use `line_through_mm`. |
| "The reply says ok, so it looks right" | Check, then look at the sheet. |
| "Two decimals everywhere" | Match the general tolerance: whole millimetres show `decimals` 0. |
| "Make a new drawing to fix one" | Edit the open drawing; ids stay, the user's work stays. |
