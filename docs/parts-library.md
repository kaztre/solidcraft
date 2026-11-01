# Parts libraries

A parts library is an ordinary folder of parts on your computer, a network share or a synced
folder (iCloud Drive, Google Drive, OneDrive). Its sub-folders are the library's categories, for
example:

```
LIBRARIES/
  PARTS LIBRARY/
    THRUSTERS/
      BLUE ROBOTICS/
        T200_THRUSTER_BR-100545/
          T200.STEP
          T200.solidcraft
    CAMERAS/
  CONNECTOR LIBRARY/
    SUBSEA CONNECTORS/
```

## The Parts Library window

**Insert ▸ Library ▸ Parts Library** in the toolbar opens it.

1. **Choose Folder…** picks the library folder. SolidCraft remembers it, so the window opens on
   it next time. **Refresh** reads the folder again after parts were added.
2. Type in the search box. Every word must appear in the part's name, its folders, its file type,
   its part number or its description, in any case; `_` in a file name counts as a space, so
   `t200 thruster` finds `T200_THRUSTER_BR-100545`. Parts whose names match come first.
3. Pick a part and press **Insert Part**, or double-click it.

What Insert Part does depends on the file:

| File | Goes into the design as |
| --- | --- |
| A SolidCraft design (`.solidcraft`) | A **linked** component (`component.insert_design`). When the library part changes, **Update Links** brings the change in. The design also keeps a copy, so it opens without the library. |
| STEP (`.step`, `.stp`), IGES (`.igs`, `.iges`), FreeCAD or Rhino | A copy of its bodies (`file.insert_step`). Later changes to the file don't reach the design. |

When a folder holds one part in several formats with one name (a vendor's `T200.STEP`,
`T200.IGS` and `T200.STL`), the window lists it once, using the design, then STEP, then IGES.
Meshes (`.stl`, `.obj`), drawings and SolidWorks files (`.sldprt`, which SolidCraft can't read)
are not listed. Hidden files and folders (names starting with `.`) and shortcuts are skipped.

## Part numbers and descriptions

A design's **part number**, **description** and **revision** (File ▸ Design Properties…) show in
the window and are searched. They also go into the bill of materials of every assembly the part is
inserted into. A STEP or IGES file has no such properties: its name and folders are what the
search finds.

## Synced and shared folders

The window lists STEP and IGES files from their names and sizes only, so a library in iCloud
Drive or Google Drive isn't downloaded just to be searched. A design's properties are read from a
small part of its file, but a synced service may still download the whole file to read it.
Inserting a part downloads it.

Linked parts are found by a path relative to the assembly that uses them (Insert Design in
docs/book/src/guide/assemblies.md), so keep the library and your projects inside one top folder and
move or sync that folder as a whole. Renaming a library folder breaks the links that pass through
it; **Relink Source (All Instances)…** points them at the new place.

## Scripts and agents

`library.parts.search {folder, query?, limit?}` lists the same parts without changing the design:
each part's path in the library, its file on this computer, name, folders, `kind` (`design` or
`cad`), file type, size and properties. Insert one with `component.insert_design` or
`file.insert_step` and the part's `file`.
