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
| `Ctrl+C` | stops a running command |
| `Ctrl+L` | collapses old output blocks |
| `Ctrl+T` | task panel |
