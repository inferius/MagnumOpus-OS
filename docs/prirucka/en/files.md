# Files and versions

A file manager with windows and icons is *coming soon*. For now you work
with files in the **Terminal** (see [Terminal for beginners](terminal.md)).

## Undoing changes

The file system remembers changes, so mistakes can be reverted:

- **`undo`** reverts the last command that changed files. You can repeat it
  up to 16 commands back.
- **`preview { ... }`** runs commands only "as a trial" (on the side), shows
  a table of changes, and takes them over only after you confirm. If you
  decline, nothing changes.
- A command that would change more than 20 files (outside `/tmp`) always
  asks first.

A trash, the version history of a single file and "try on the side" in the
file manager window are *coming soon*.
