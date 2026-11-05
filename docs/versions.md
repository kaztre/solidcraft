# Versions

SolidCraft keeps versions of the files in a project folder: designs, STEP files, drawings as PDF,
KiCad projects and documents. You check in files when they're ready, see who changed what and when, release a part as a revision, and put back an
older version. Single parts and assemblies work the same way, and nothing needs to be installed
or typed.

## The Versions tab

The turn-back arrow in the Browser panel's header (**Show Versions**) switches the panel to
**Versions**; the same button switches back to the Browser. The Versions tab follows the design
in front:

- **This part** (or **This assembly**: the assembly and the linked parts it uses from the same
  folder) shows only that design's changes and history.
- **Whole folder** shows everything in the folder.

Preferences ▸ Design ▸ **Show the Versions tab in the Browser** takes the button away.

## Keeping versions of a folder

Open a design and switch to Versions. If its folder doesn't keep versions yet, press **Keep
Versions of "FOLDER"**, or **Keep Versions of Another Folder…** to choose one, such as your
library's or project's top folder. Every design (`.solidcraft`) and STEP file in the folder and
its sub-folders then gets a history, whatever it is: designs, STEP files, PDFs, KiCad projects,
documents and pictures. Other programs' scratch is left out: hidden files, lock files (`~$report.docx`),
backups (`.bak`, a KiCad project's `-backups` folder, autosaves), temporary files, KiCad's
per-person settings (`.kicad_prl`) and caches, and build folders. The assistant reads the same
folder ([the project folder](agent-chat.md#the-project-folder)).

The history is kept on this computer, in SolidCraft's data folder (on a Mac, `~/Library/
Application Support/SolidCraft/versions`), not inside your folder: a folder synced by iCloud
Drive, Google Drive or OneDrive would upload it and can damage it. A folder that already has its
own `.git` (cloned with git) uses that one.

## Checking in

Save your changes, then on the Versions tab:

1. **My changes** lists the files that differ from their last version: new, changed or deleted.
   Untick any you don't want in this check-in.
2. Type what changed in the box. It's shown in the history.
3. Press **Check In N Files**.

A design with unsaved changes says so, with a **Save** button. A file that iCloud Drive keeps
only in the cloud isn't downloaded just to look at it; it shows "in iCloud", and checking it in
downloads it.

Check-ins are made under your computer's user name; **Change Name** sets the name the history
shows.

## Revisions of a part

With a single part in front, **Release as Rev B** under the message box releases the part as
that revision with the check-in. Its Revision property (File ▸ Design Properties…) is set and
saved first, so the bill of materials of every assembly that uses it shows it. The next revision
follows the last one: Rev A, Rev B, Rev C…

You can also release an earlier check-in: pick it in the history and press **Release NAME as
Rev X…**. Revisions are saved in the history and never move.

## History

**History** lists the versions newest first: the message, who checked in, when, and the
revisions and names given to it. Pick one to see the files it changed and:

- **Open This Version** opens a copy of the file as it was: a design or CAD file in a new tab, to look
  at or measure; a PDF, KiCad file or document in the program your computer opens it with.
  Saving the copy doesn't change anything in the folder.
- **Restore NAME to This Version…** puts the part back as it was. It then shows in My changes;
  check it in to keep it. Newer versions stay in the history, so a restore can itself be undone
  by restoring again. A part open with unsaved changes has to be saved first.
- **Save as New Part…** writes that version as a new part beside it. Its first check-in notes
  where it came from ("Copied from …").
- In This assembly and Whole folder: **Restore Whole Folder to This Version…** puts every part
  back (parts added since stay), and **Name This Version…** names the whole folder at that
  point, such as "v1.3" or "Sea trial 2".

## Sharing with a team

This first step keeps versions on one computer. Sharing them through a private GitHub library
(Sign in with GitHub, Get Latest, and check-out so two people don't change one part at once) is
the next step; the history is already in git's own format, so nothing has to change when it
comes.

## Scripts and agents

`versions.start {folder}` starts a folder's history and `versions.status {folder}` lists what a
check-in would save. `versions.check_in {folder, message, paths, author?, release?: {path,
revision}}` saves a version, `versions.history {folder, path?, limit?}` lists them,
`versions.restore {folder, commit, path?}` puts parts back, `versions.copy {folder, commit, path,
to?}` writes an old version to a new file, and `versions.name {folder, commit, version? | path
and revision}` names a version or releases a revision. Paths are relative to the folder, with `/`
between folders. None of them change the open design.
