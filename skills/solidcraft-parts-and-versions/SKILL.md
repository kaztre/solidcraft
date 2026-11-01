---
name: solidcraft-parts-and-versions
description: "Use when someone asks how to set up or use a parts library, insert a library part, keep linked parts up to date, or check in, see the history of, release a revision of, or go back to an older version of their parts in SolidCraft, or wants that done for them through the MCP server."
---

# Parts libraries and versions

## Teach, one step at a time
- "How do I…" means they want to learn the buttons. Answer with a short numbered list using the exact names on screen in **bold**, then offer to do it for them.
- No command lines, git words or file paths unless they ask. Say "check in", "version", "revision", never "commit", "repo" or "tag".
- If they're stuck, ask what they see, or look with `screenshot {}`.
- Ask before anything that changes files: a check-in (they write the message), a restore (it replaces the file), converting a whole category.

## Where things are
- **Library** button: Insert tab (Library group) and Assemble tab (Components group). It opens the Parts Library window.
- **Show Versions**: the turn-back arrow at the top right of the Browser panel. The same button goes back to the Browser.
- Linked parts: a changed project part comes into the assembly by itself (Undo takes it back). A changed library part shows an **Update available** card with **Update** and **Not Now**. Preferences ▸ Design ▸ **Update changed linked parts automatically**.

## The lessons people ask for
1. **Set up a library:** Library ▸ **Add Library…** ▸ pick the top folder. Its sub-folders become categories in the tree. Search box, **Pictures** or **List**, then **Insert Part** or double-click.
2. **STEP parts:** inserting one saves a SolidCraft part beside it with its picture, then links it. **Convert N to SolidCraft Parts…** does a whole category; try a small one first.
3. **Start keeping versions:** open a saved part ▸ **Show Versions** ▸ **Keep Versions of "folder"**. Every design and STEP file in that folder and its sub-folders gets a history.
4. **Check in:** save, type what changed, untick anything to leave out under **My changes**, press **Check In**.
5. **Release a revision:** with the part open, tick **Release as Rev B** before **Check In**. It also sets the part's Revision (shown in bills of materials).
6. **Go back:** click a version in the history ▸ **Open This Version** to look, **Restore … to This Version…** to put it back (then check it in), **Save as New Part…** to branch off a copy.
7. **Whole project:** switch **This part** to **Whole folder** for everything, **Restore Whole Folder to This Version…** and **Name This Version…** ("v1.3").

## Facts to get right
- The history is kept on this computer, outside the folder, so iCloud or Google Drive syncing can't damage it. Sharing with a team through GitHub isn't built yet; say so if asked.
- Only designs and STEP files are kept; meshes, PDFs and drawings are ignored.
- A part only in iCloud shows "in iCloud"; checking it in downloads it.
- Library parts never change an assembly unasked; project parts do.

## Doing it for them
The `versions.*` and `library.parts.*` commands do everything the buttons do, without changing the open design. Their parameters, replies and a worked session: `skill_reference {"skill":"solidcraft-parts-and-versions"}`. Afterwards, tell them where to look in the window to see what you did.
