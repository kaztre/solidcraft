# Parts libraries

A parts library is an ordinary folder of parts on your computer, a network share or a synced
folder (iCloud Drive, Google Drive, OneDrive). Its sub-folders are the library's categories, for
example:

```
ROV PARTS/
  THRUSTERS/
    BLUE ROBOTICS/
      T200_THRUSTER_BR-100545/
        T200.STEP
        T200.solidcraft
  CAMERAS/
  SUBSEA CONNECTORS/
```

You can keep several libraries: a team library everyone shares, your own parts, vendor downloads.

## The Parts Library window

**Library** in the toolbar opens it: on the **Insert** tab (Library group) and the **Assemble**
tab (Components group).

- **Add Library…** picks a folder and adds it to the list, named after the folder. SolidCraft
  remembers the list. **Remove Library** takes the library picked in the tree out of the list;
  its files stay where they are. **Refresh** reads the libraries again after parts were added.
- The **tree** on the left lists **All libraries**, each library and its folders, with how many
  parts each holds. Pick one to see only its parts; the arrow opens and closes a folder.
- **Pictures** or **List**: pictures of the parts, or a list with each part's part number, folder
  and file type.
- Type in the **search** box. Every word must appear in the part's name, its folders, its file
  type, its part number or its description, in any case; `_` in a file name counts as a space, so
  `t200 thruster` finds `T200_THRUSTER_BR-100545`. Parts whose names match come first.
- Pick a part and press **Insert Part**, or double-click it.

A part alone in a vendor's folder named after it (`M10_TO_M14_BULKHEAD_ADAPTER_BR-102909/
BR-102512_RevA.STEP`) is listed under the folder's name. When a folder holds one part in several
formats with one name (`T200.STEP`, `T200.IGS`, `T200.STL`), the window lists it once: the
SolidCraft part, then STEP, then IGES. Meshes (`.stl`, `.obj`), drawings and SolidWorks files
(`.sldprt`, which SolidCraft can't read) are not listed. Hidden files and folders (names starting
with `.`) and shortcuts are skipped.

## Linked library parts

**Insert Part** always links the part (`component.insert_design`): when the library part changes,
**Update Links** brings the change into every assembly that uses it. The assembly also keeps a
copy, so it opens without the library.

A STEP or IGES part is first saved as a SolidCraft part beside it, with the same name
(`T200.STEP` → `T200.solidcraft`), then linked. From then on the library lists the SolidCraft
part instead of the STEP, which stays as the vendor's original. The new part gets:

- its **name** and **description** from the name the window shows,
- a **part number** found in the names, such as `BR-102909` (one to five letters, a hyphen and at
  least three digits),
- its **picture**, saved in the file,
- and, with **Stand up parts exported with Y up** on (the default), a quarter turn about X when
  the STEP file says it came from a Y-up program such as SolidWorks.

Change the properties later with File ▸ Design Properties… in the part. If the part can't be
saved in the library (a folder you can't write to), a copy of the STEP is inserted instead, and
the window says why.

## Changed parts

A linked part remembers the file it was read from. While a design with linked parts is open,
SolidCraft looks at their files every couple of seconds, and again when you bring another design
to the front:

- A **project part** whose file changed (you saved it in another tab, or a teammate's copy
  arrived) comes into the design by itself, as one step that **Undo** takes back. The status
  line says which parts were updated. It waits while a dialog, a sketch or a tool is open.
- A **library part** (one inside a Parts Library folder) stays as it is: an **Update available**
  notice lists it, and **Update** brings in the new revision. **Not Now** leaves it until its file
  changes again. A vendor's new revision never changes a design without you asking.

Preferences ▸ Design ▸ **Update changed linked parts automatically** turns the first off: then
every changed part waits for **Update**, and **Update Links** still reads them all again. A link
made before SolidCraft remembered its file is checked once it has been updated.

## Converting a whole category

Pick a library or a folder in the tree and press **Convert N to SolidCraft Parts…**: it saves every
STEP and IGES part there as a SolidCraft part, as above, one after another beside the window. You
can keep working; **Stop** ends it after the part in hand. Parts already converted are left alone,
so converting again carries on where it stopped. At the end the window lists the parts that
couldn't be converted and why (a file over the 512 MB import limit, a file SolidCraft can't read).
A solid whose faces SolidCraft can't make into a solid, nor sew together, still comes in: as a
surface body of the file's own faces, each its own sheet. It looks and measures as the part does
and can be placed and linked, but it can't be filleted, shelled or cut; the import's notes say
which solid and why.

Converting downloads every file a synced folder keeps only in the cloud, and large assemblies take
minutes each, so try one category first.

**Reconvert N…** makes the parts already converted in the category again from the STEP and IGES
files beside them, with this release's import: parts converted by an earlier release get today's
geometry and their picture, and open faster. Each part keeps its file name (a `.solidcraft` part
stays `.solidcraft`), part number, description and revision, so the designs that link it still
find it and offer **Update**. A part with no CAD file beside it is left as it is, and so is a part
someone has added features to since it was converted: it is listed with the reason.

## Pictures

A SolidCraft part shows the picture saved in it; one saved without a picture (converted by an
earlier release) has one drawn, as a STEP or IGES part does. A STEP or IGES part's picture is drawn
once and kept on this computer; the part's file is never changed for it. So that a library in iCloud Drive or Google Drive isn't downloaded just to be
browsed, a picture is drawn by itself only for a file already on this computer and up to 30 MB;
the others show a download sign until you click them. Converting a category gives every part its
picture, for everyone who uses the library.

## Part numbers and descriptions

A part's **part number**, **description** and **revision** (File ▸ Design Properties…) show in
the list and are searched. They also go into the bill of materials of every assembly the part is
inserted into.

## Synced and shared folders

Linked parts are found by a path relative to the assembly that uses them (Insert Design in
docs/book/src/guide/assemblies.md), so keep the libraries and your projects inside one top folder
and move or sync that folder as a whole. Renaming a library folder breaks the links that pass
through it; **Relink Source (All Instances)…** points them at the new place.

## Scripts and agents

`library.parts.search {folder, query?, limit?}` lists a library's parts without changing the
design: each part's path in the library, its file on this computer, name, folders, `kind`
(`design` or `cad`), file type, size and properties. `library.parts.convert {path, name?,
part_number?, description?, organization?, stand_up?, overwrite?}` saves a CAD file as a library
part beside it, without changing the open design. Link one with `component.insert_design` and its
`file`.
