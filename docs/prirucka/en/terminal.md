# Terminal for beginners

The terminal is a window where you type commands. The result appears as a
block under the command, often as a clear table.

## Help

```text
help
help ls
```

`help` lists all commands, `help ls` describes one of them. The full list
of commands is in the developer section: [Shell commands](../../en/shell-commands.md).
When a command ends with an error, type `explain` and the error code: you
will learn what it means and what to do.

## A few useful commands

| Command | What it does |
|---|---|
| `ls` | shows the files in the current folder |
| `cd Dokumenty` | goes to the folder Dokumenty |
| `pwd` | tells you which folder you are in |
| `open poznamky.txt` | prints the content of a file |
| `mkdir Projekty` | creates a folder |
| `cp a.txt b.txt`, `mv a.txt b.txt`, `rm a.txt` | copy, move, delete |
| `undo` | reverts the last change to files |
| `history` | earlier commands |

Example: large files, biggest first:

```text
ls ~/Dokumenty -r | where size > 100MB | sort size -d
```

## Do not be afraid of mistakes

- Every command that changes files can be reverted with **`undo`**
  (the last 16 commands).
- When a command would change more than 20 files, the terminal asks first.
- `preview { ... }` shows what would happen and makes the changes only
  after you confirm.

## Controls

| Key | What it does |
|---|---|
| `Tab` | completes a command, a path or a column name |
| `↑` `↓` | previous and next command |
| `Ctrl+C` | copies the selected text; without a selection stops a running command |
| `Ctrl+V` | pastes text from the clipboard into the line (runs nothing) |
| `Ctrl+L` | collapses old output blocks |
| `Ctrl+T` | task panel |

When there are several options, Tab opens a suggestion list above the
line. Typing narrows it, arrows choose, Enter or Tab completes the item,
Esc closes the list. You can also click an item.

With the mouse: drag to select text, double click for a word, triple
click for a whole line. A click into the command line moves the cursor.
The "Copy output" button in a block header copies the whole command output.
