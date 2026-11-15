# Shell commands

> This page is generated from the command metadata in `libs/shlang/src/doc/` (`cargo xtask docs`) - do not edit it by hand. `help <command>` in the terminal shows the same data and the examples are tests: they run in the test folder `/home` (files `a.txt`, `b.txt`, `data.json` and the folder `docs`) and their output is compared.

## Index

- **Files:** [`ls`](#ls), [`cd`](#cd), [`pwd`](#pwd), [`open`](#open), [`start`](#start), [`save`](#save), [`rm`](#rm), [`cp`](#cp), [`mv`](#mv), [`mkdir`](#mkdir), [`exists`](#exists), [`df`](#df)
- **Text and output:** [`echo`](#echo), [`log`](#log), [`lines`](#lines), [`split`](#split), [`join`](#join), [`trim`](#trim), [`upper`](#upper), [`lower`](#lower), [`contains`](#contains), [`starts-with`](#starts-with), [`ends-with`](#ends-with), [`replace`](#replace)
- **Lists and tables:** [`where`](#where), [`each`](#each), [`select`](#select), [`sort`](#sort), [`first`](#first), [`last`](#last), [`count`](#count), [`sum`](#sum), [`group`](#group), [`reverse`](#reverse), [`range`](#range)
- **Conversions and time:** [`from`](#from), [`to`](#to), [`convert`](#convert), [`describe`](#describe), [`now`](#now), [`sleep`](#sleep)
- **Tasks:** [`tasks`](#tasks), [`jobs`](#jobs), [`fg`](#fg), [`wait`](#wait), [`kill`](#kill), [`cancel`](#cancel)
- **Session and safety:** [`undo`](#undo), [`preview`](#preview), [`history`](#history), [`run`](#run), [`error`](#error), [`exit`](#exit)
- **Help:** [`help`](#help), [`explain`](#explain)
- **System:** [`audio`](#audio), [`vzhled`](#vzhled), [`povoleni`](#povoleni), [`loglevel`](#loglevel), [`report`](#report), [`install`](#install), [`shutdown`](#shutdown), [`reboot`](#reboot)

## Files

### `ls`

lists files in a folder as a table

Lists a folder as a table with name, path, kind, size and modification time. The output is a table, so it can be filtered and sorted further in a pipeline. Hidden files (name starting with a dot) appear only with `-a`. For a folder `size` is the logical size (the sum of files in the subtree, known to the FS at once without walking it) and `items` its content (`files` and `dirs` separately); the space really used (shared blocks, deduplication, compression) will be shown by the upcoming `disk` / `fs info` command. Without the right to manage the folder the summary is missing.

```shell
ls [path] [-r] [-a]
```

Full name `file-list` · input: nothing · output: table (name, path, kind, size, mtime, ext, access, dataset, items, files, dirs)

| Argument | Type | Default | Description |
|---|---|---|---|
| `[path]` | path | pracovní složka | folder to list (default: working folder) |
| `-r, --recursive` | switch |  | include subfolders |
| `-a, --all` | switch |  | include hidden files (starting with a dot) |

Name, size and the content of a folder.

```shell
ls | select name size items
```

```text
name       size    items
---------  ------  ---------
a.txt         5 B
b.txt        10 B
data.json    49 B
docs       3.0 KB  2 soubory
```

Names of files in the working folder.

```shell
ls | select name
```

```text
name
---------
a.txt
b.txt
data.json
docs
```

Only large files in the docs folder.

```shell
ls docs | where size > 1KB | select name
```

```text
name
-------
big.pdf
```

**Needs:** read access to the folder

**Errors:** `NOT_FOUND`, `ACCESS_DENIED`

**Related:** [`cd`](#cd), [`where`](#where), [`sort`](#sort), [`select`](#select)


### `cd`

changes the working folder (no argument: home)

Changes the session's working folder. Relative paths in later commands are resolved from it; the terminal prompt shows it right away. Without an argument it goes to the home folder (`~`), `..` is the parent folder. The session remembers recent folders: `cd -` goes back to the previous one, `cd -2` two back, `cd +` forward again and `cd --history` lists them. In the terminal window Alt+Left and Alt+Right do the same when the line is empty.

```shell
cd [path] [--history]
```

Full name `dir-change` · input: nothing · output: any value

| Argument | Type | Default | Description |
|---|---|---|---|
| `[path]` | path | domovská složka | target folder (default: home folder); `-`, `-N` back, `+` forward |
| `--history` | switch |  | recent folders of the session (number for `cd -N`) |

Goes to the docs folder and prints it.

```shell
cd docs; pwd
```

```text
/home/docs
```

Back to the previous folder.

```shell
cd docs; cd /tmp; cd -; pwd
```

```text
/home/docs
```

Recent folders of the session, newest first.

```shell
cd docs; cd /tmp; cd --history | select path
```

```text
path
----------
/home/docs
/home
```

**Errors:** `NOT_FOUND`, `SH_TYPE`, `SH_ARGS`

**Related:** [`pwd`](#pwd), [`ls`](#ls)


### `pwd`

returns the working folder

Returns the session's working folder as text.

```shell
pwd
```

Full name `dir-current` · input: nothing · output: text

The current folder.

```shell
pwd
```

```text
/home
```

**Related:** [`cd`](#cd)


### `open`

reads a file as bytes

Reads a file as bytes. `from text` turns them into text (no hidden conversion), `from json` into values. `open` does not read a file over 64 KiB whole: it returns a reference to the file (path, size, kind by the start of the content, e.g. a BMP image 1920×1080) and the content is read only by commands that need it (`from text`, `lines`, `save`...). The terminal shows such a file as a summary; the fields `size`, `kind` and `path` can be read directly (`(open img.bmp).size`).

```shell
open path
```

Full name `file-read` · input: nothing · output: bytes

| Argument | Type | Default | Description |
|---|---|---|---|
| `path` | path |  | file to read |

The file content as text.

```shell
open a.txt | from text
```

```text
alfa
```

Records from JSON filtered in a pipeline.

```shell
open data.json | from json | where age > 18
```

```text
name  age
----  ---
Eva    31
```

**Needs:** read access to the file

**Errors:** `NOT_FOUND`, `ACCESS_DENIED`

**Related:** [`save`](#save), [`from`](#from), [`lines`](#lines)


### `start`

opens a file or folder in an application by its type (like a double click)

Opens a file or folder in an application by its type, like a double click in Files (and `start` on Windows). The "open a path" service picks the application from the associations in application manifests (`[open]`). A file no application opens is shown by Files; a folder opens in Files. A bare path to a file on the command line does the same, only without Files: with no application it shows a summary like `open`. `open` reads the file content, not `start`. The text console (no graphical interface) has no such service and `start` fails with `NOT_SUPPORTED`.

```shell
start path
```

Full name `file-start` · input: nothing · output: nothing

| Argument | Type | Default | Description |
|---|---|---|---|
| `path` | path |  | file or folder |

Opens the text in the editor.

```shell
start zprava.txt
```

Opens the folder in Files.

```shell
start ~/Obrázky
```

**Needs:** the graphical interface (the "open a path" service)

**Errors:** `NOT_FOUND`, `NOT_SUPPORTED`, `SH_OPEN`

**Related:** [`open`](#open), [`ls`](#ls)


### `save`

saves bytes or text to a file (records only after `to text`/`to json`)

Saves the pipeline input to a file. Only bytes and text are saved; records must be converted first (`to json`, `to text`). The file is replaced at once: on failure the original content stays. `undo` reverts the change.

```shell
save path [-a]
```

Full name `file-write` · input: any value · output: nothing

| Argument | Type | Default | Description |
|---|---|---|---|
| `path` | path |  | target file |
| `-a, --append` | switch |  | append to the end |

Saves text and reads it back.

```shell
echo ahoj | save /tmp/x.txt; open /tmp/x.txt | from text
```

```text
ahoj
```

A table into a JSON file.

```shell
ls | select name | to json | save /tmp/seznam.json
```

**Needs:** write access to the folder

**Errors:** `SH_TYPE`, `ACCESS_DENIED`, `NO_SPACE`

**Related:** [`open`](#open), [`to`](#to), [`undo`](#undo)


### `rm`

deletes files

Deletes files (folders only with `-r`). `undo` reverts the deletion; safer is `preview { rm ... }` first.

```shell
rm ... [-r]
```

Full name `file-remove` · input: nothing · output: nothing

| Argument | Type | Default | Description |
|---|---|---|---|
| `...` | path |  | files to delete |
| `-r, --recursive` | switch |  | also folders with their content |

Deletes a file and checks it is gone.

```shell
rm a.txt; exists a.txt
```

```text
false
```

**Needs:** write access to the folder

**Errors:** `NOT_FOUND`, `ACCESS_DENIED`, `SH_ARGS`

**Related:** [`preview`](#preview), [`undo`](#undo), [`mv`](#mv)


### `cp`

copies a file

Copies a file (a folder with `-r`). The target may be a folder: the copy keeps the same name.

```shell
cp from to [-r]
```

Full name `file-copy` · input: nothing · output: nothing

| Argument | Type | Default | Description |
|---|---|---|---|
| `from` | path |  | source |
| `to` | path |  | target (file or folder) |
| `-r, --recursive` | switch |  | also folders |

A copy and its content.

```shell
cp a.txt /tmp/kopie.txt; open /tmp/kopie.txt | from text
```

```text
alfa
```

**Needs:** read access to the source, write access to the target

**Errors:** `NOT_FOUND`, `ACCESS_DENIED`

**Related:** [`mv`](#mv), [`rm`](#rm)


### `mv`

moves or renames a file

Moves or renames a file or folder. Within one file system it is instant.

```shell
mv from to
```

Full name `file-move` · input: nothing · output: nothing

| Argument | Type | Default | Description |
|---|---|---|---|
| `from` | path |  | source |
| `to` | path |  | new name or target folder |

Renaming.

```shell
mv a.txt c.txt; exists c.txt
```

```text
true
```

**Needs:** write access to both folders

**Errors:** `NOT_FOUND`, `ACCESS_DENIED`, `ALREADY_EXISTS`

**Related:** [`cp`](#cp), [`rm`](#rm)


### `mkdir`

creates a folder

Creates a folder; with `-p` also the missing parent folders (and it is fine if it exists).

```shell
mkdir ... [-p]
```

Full name `dir-create` · input: nothing · output: nothing

| Argument | Type | Default | Description |
|---|---|---|---|
| `...` | path |  | folders to create |
| `-p, --parents` | switch |  | create parent folders too |

A whole path at once.

```shell
mkdir -p /tmp/a/b; exists /tmp/a/b
```

```text
true
```

**Needs:** write access to the parent folder

**Errors:** `ALREADY_EXISTS`, `NOT_FOUND`, `ACCESS_DENIED`

**Related:** [`ls`](#ls), [`rm`](#rm)


### `exists`

does a file exist?

Returns `true` when the path exists (file or folder), otherwise `false`. Useful in `if`.

```shell
exists path
```

Full name `file-exists` · input: nothing · output: any value

| Argument | Type | Default | Description |
|---|---|---|---|
| `path` | path |  | path |

Branching on existence.

```shell
if (exists a.txt) { echo je } else { echo neni }
```

```text
je
```

**Related:** [`ls`](#ls), [`open`](#open)


### `df`

space on the file system

Total, used and free space of the file system the path is on.

```shell
df [path]
```

Full name `fs-info` · input: nothing · output: table (total, used, free)

| Argument | Type | Default | Description |
|---|---|---|---|
| `[path]` | path | pracovní složka | a path on the file system (default: working folder) |

Only the free space.

```shell
df | select free
```

**Errors:** `NOT_FOUND`

**Related:** [`ls`](#ls)


## Text and output

### `echo`

returns its arguments joined by a space

Returns the arguments joined by a space as text. An expression in `{...}` inside quotes is substituted.

```shell
echo ...
```

Full name `text-print` · input: nothing · output: text

| Argument | Type | Default | Description |
|---|---|---|---|
| `...` | value |  | values to print |

Text from arguments.

```shell
echo ahoj světe
```

```text
ahoj světe
```

Variable substitution.

```shell
let n = 3; echo "je jich {n}"
```

```text
je jich 3
```

**Related:** [`log`](#log), [`join`](#join)


### `log`

prints a note to the terminal

Prints a note to the terminal (muted) but returns nothing into the pipeline - handy in scripts.

```shell
log ...
```

Full name `text-log` · input: nothing · output: nothing

| Argument | Type | Default | Description |
|---|---|---|---|
| `...` | value |  | note text |

A note without output.

```shell
log začínám
```

**Related:** [`echo`](#echo)


### `lines`

splits text into lines

Splits text (or UTF-8 bytes) into a list of lines. `\r\n` counts as one line ending.

```shell
lines
```

Full name `text-lines` · input: any value · output: list

Number of lines in a file.

```shell
open b.txt | lines | count
```

```text
1
```

**Errors:** `SH_TYPE`, `SH_FORMAT`

**Related:** [`split`](#split), [`join`](#join), [`open`](#open)


### `split`

splits text

Splits text by a separator into a list.

```shell
split sep
```

Full name `text-split` · input: text · output: list

| Argument | Type | Default | Description |
|---|---|---|---|
| `sep` | text |  | separator |

A list of three parts.

```shell
"a,b,c" | split ","
```

```text
a
b
c
```

**Errors:** `SH_TYPE`

**Related:** [`join`](#join), [`lines`](#lines)


### `join`

joins a list of texts

Joins a list of texts into one, optionally with a separator.

```shell
join [sep]
```

Full name `text-join` · input: any value · output: text

| Argument | Type | Default | Description |
|---|---|---|---|
| `[sep]` | text | "" | separator |

Joined with a dash.

```shell
["a", "b"] | join "-"
```

```text
a-b
```

**Errors:** `SH_TYPE`

**Related:** [`split`](#split), [`echo`](#echo)


### `trim`

removes spaces at the edges

Removes spaces and line endings at the start and end of text.

```shell
trim
```

Full name `text-trim` · input: text · output: text

Text without edge spaces.

```shell
"  x  " | trim
```

```text
x
```

**Errors:** `SH_TYPE`

**Related:** [`replace`](#replace)


### `upper`

upper case

Converts text to upper case (including diacritics).

```shell
upper
```

Full name `text-upper` · input: text · output: text

Upper case.

```shell
"čaj" | upper
```

```text
ČAJ
```

**Errors:** `SH_TYPE`

**Related:** [`lower`](#lower)


### `lower`

lower case

Converts text to lower case (including diacritics).

```shell
lower
```

Full name `text-lower` · input: text · output: text

Lower case.

```shell
"ČAJ" | lower
```

```text
čaj
```

**Errors:** `SH_TYPE`

**Related:** [`upper`](#upper)


### `contains`

does the text (or list) contain the part?

For text tells whether it contains a substring, for a list whether it contains an item.

```shell
contains part
```

Full name `text-contains` · input: any value · output: any value

| Argument | Type | Default | Description |
|---|---|---|---|
| `part` | value |  | the part or item to look for |

A substring.

```shell
"beta" | contains "et"
```

```text
true
```

**Errors:** `SH_TYPE`

**Related:** [`starts-with`](#starts-with), [`ends-with`](#ends-with), [`where`](#where)


### `starts-with`

does it start with the text?

Tells whether the text starts with the given text.

```shell
starts-with prefix
```

Full name `text-starts-with` · input: text · output: any value

| Argument | Type | Default | Description |
|---|---|---|---|
| `prefix` | text |  | prefix |

The start of a name.

```shell
"report.pdf" | starts-with "rep"
```

```text
true
```

**Errors:** `SH_TYPE`

**Related:** [`ends-with`](#ends-with), [`contains`](#contains)


### `ends-with`

does it end with the text?

Tells whether the text ends with the given text (an extension, say).

```shell
ends-with suffix
```

Full name `text-ends-with` · input: text · output: any value

| Argument | Type | Default | Description |
|---|---|---|---|
| `suffix` | text |  | suffix |

An extension.

```shell
"report.pdf" | ends-with ".pdf"
```

```text
true
```

**Errors:** `SH_TYPE`

**Related:** [`starts-with`](#starts-with), [`contains`](#contains)


### `replace`

replaces all occurrences

Replaces all occurrences of one text with another.

```shell
replace from to
```

Full name `text-replace` · input: text · output: text

| Argument | Type | Default | Description |
|---|---|---|---|
| `from` | text |  | what to replace |
| `to` | text |  | replacement |

All dashes.

```shell
"a-b-c" | replace "-" "+"
```

```text
a+b+c
```

**Errors:** `SH_TYPE`

**Related:** [`trim`](#trim), [`split`](#split)


## Lists and tables

### `where`

keeps the rows that match a condition

Keeps only the rows that match a condition. The row's columns are plain names in the condition (`size > 1MB`), the whole row is `it`.

```shell
where cond
```

Full name `list-filter` · input: any value · output: any value

| Argument | Type | Default | Description |
|---|---|---|---|
| `cond` | condition |  | condition over a row |

Only text files.

```shell
ls | where ext == "txt" | select name
```

```text
name
-----
a.txt
b.txt
```

**Errors:** `SH_TYPE`, `SH_NO_MATCH`

**Related:** [`select`](#select), [`sort`](#sort), [`each`](#each)


### `each`

applies a function to every row

Applies a function to every row and returns the list of results. With `--parallel N` at most N rows run concurrently as tasks.

```shell
each f [--parallel hodnota]
```

Full name `list-map` · input: any value · output: any value

| Argument | Type | Default | Description |
|---|---|---|---|
| `f` | closure |  | function `x => ...` |
| `-p, --parallel` | number | 1 | at most N rows concurrently (tasks) |

Each item times ten.

```shell
[1, 2, 3] | each x => x * 10
```

```text
10
20
30
```

**Errors:** `SH_TYPE`

**Related:** [`where`](#where), [`tasks`](#tasks)


### `select`

keeps only the chosen columns

Keeps only the chosen columns of a table (in the given order).

```shell
select ...
```

Full name `record-select` · input: any value · output: any value

| Argument | Type | Default | Description |
|---|---|---|---|
| `...` | text |  | column names |

Name and size (for a folder the subtree sum).

```shell
ls | select name size
```

```text
name       size
---------  ------
a.txt         5 B
b.txt        10 B
data.json    49 B
docs       3.0 KB
```

**Errors:** `SH_TYPE`

**Related:** [`where`](#where), [`sort`](#sort)


### `sort`

sorts the rows (by a column)

Sorts the rows ascending, by a column or by the value itself; `-d` descending.

```shell
sort [by] [-d]
```

Full name `list-sort` · input: any value · output: any value

| Argument | Type | Default | Description |
|---|---|---|---|
| `[by]` | text | hodnota | column (default: the value) |
| `-d, --desc` | switch |  | descending |

Largest first.

```shell
ls docs | sort size -d | select name
```

```text
name
---------
big.pdf
small.pdf
```

**Errors:** `SH_TYPE`

**Related:** [`first`](#first), [`reverse`](#reverse), [`group`](#group)


### `first`

first row (or first n)

Without an argument returns the first item, with a number a list of the first n.

```shell
first [n]
```

Full name `list-first` · input: any value · output: any value

| Argument | Type | Default | Description |
|---|---|---|---|
| `[n]` | number | 1 | how many |

The first item.

```shell
[5, 6, 7] | first
```

```text
5
```

**Errors:** `SH_TYPE`

**Related:** [`last`](#last), [`sort`](#sort)


### `last`

last row (or last n)

Without an argument returns the last item, with a number a list of the last n.

```shell
last [n]
```

Full name `list-last` · input: any value · output: any value

| Argument | Type | Default | Description |
|---|---|---|---|
| `[n]` | number | 1 | how many |

The last two.

```shell
[5, 6, 7] | last 2
```

```text
6
7
```

**Errors:** `SH_TYPE`

**Related:** [`first`](#first), [`reverse`](#reverse)


### `count`

number of rows (or characters of text)

Number of list items or characters of text.

```shell
count
```

Full name `list-count` · input: any value · output: any value

How many files there are.

```shell
ls | count
```

```text
4
```

**Errors:** `SH_TYPE`

**Related:** [`sum`](#sum), [`where`](#where)


### `sum`

sum (of a column)

Adds up numbers, sizes or durations; for a table the given column.

```shell
sum [field]
```

Full name `list-sum` · input: any value · output: any value

| Argument | Type | Default | Description |
|---|---|---|---|
| `[field]` | text | hodnota | column (default: the value) |

Total size of a folder's files.

```shell
ls docs | sum size
```

```text
3.0 KB
```

**Errors:** `SH_TYPE`, `SH_ARITH`

**Related:** [`count`](#count), [`group`](#group)


### `group`

groups the rows by a column

Groups the rows by a column value into a table with the columns `key` and `items`.

```shell
group by
```

Full name `list-group` · input: any value · output: table (key, items)

| Argument | Type | Default | Description |
|---|---|---|---|
| `by` | text |  | column |

Kinds of entries in a folder.

```shell
ls | group kind | select key
```

```text
key
----
file
dir
```

**Errors:** `SH_TYPE`

**Related:** [`sort`](#sort), [`sum`](#sum)


### `reverse`

reverses the order

Reverses the order of list items.

```shell
reverse
```

Full name `list-reverse` · input: any value · output: any value

Backwards.

```shell
[1, 2, 3] | reverse
```

```text
3
2
1
```

**Errors:** `SH_TYPE`

**Related:** [`sort`](#sort)


### `range`

numbers from..to (upper bound excluded)

A list of integers from `from` to `to` (the upper bound is excluded).

```shell
range from to
```

Full name `list-range` · input: nothing · output: any value

| Argument | Type | Default | Description |
|---|---|---|---|
| `from` | number |  | first number |
| `to` | number |  | bound (excluded) |

The sum 1 + 2 + 3.

```shell
range 1 4 | sum
```

```text
6
```

**Errors:** `SH_TYPE`

**Related:** [`each`](#each)


## Conversions and time

### `from`

converts bytes/text into values (json, text, lines)

Converts bytes or text into values: `json` into records and lists, `text` into text (UTF-8), `lines` into a list of lines. No conversion happens implicitly.

```shell
from format
```

Full name `data-from` · input: any value · output: any value

| Argument | Type | Default | Description |
|---|---|---|---|
| `format` | text |  | json, text or lines |

JSON as a table.

```shell
open data.json | from json | select name
```

```text
name
----
Eva
Jan
```

**Errors:** `SH_FORMAT`, `SH_TYPE`

**Related:** [`to`](#to), [`open`](#open)


### `to`

converts values into text (json, text)

Converts values into text: `json` (machine readable), `text` (as in the terminal). Needed before `save` for records.

```shell
to format
```

Full name `data-to` · input: any value · output: any value

| Argument | Type | Default | Description |
|---|---|---|---|
| `format` | text |  | json or text |

A list as JSON.

```shell
[1, 2] | to json
```

```text
[
  1,
  2
]
```

**Errors:** `SH_FORMAT`

**Related:** [`from`](#from), [`save`](#save)


### `convert`

converts units, number bases, text encodings and time zones

A universal conversion. Units from the unit table: `1GiB | convert MB` returns a number in the target unit, a number without a unit converts with `--from unit`. Bases `hex`, `bin`, `oct`, `dec`: an integer to text with a prefix (`0xff`) and text with a number back (`"ff" | convert dec --from hex`). Encodings `utf8`, `utf16le`, `utf16be`, `latin2` turn text into bytes, `base64` and `hex` bytes into text; back with `--from encoding`. A time converts to a time zone (an IANA name or an offset `UTC+2`) as local time with daylight saving. Data formats (json, lines) are `from` and `to`.

```shell
convert [target] [--from hodnota]
```

Full name `value-convert` · input: any value · output: any value

| Argument | Type | Default | Description |
|---|---|---|---|
| `[target]` | text |  | target unit, base, encoding or time zone |
| `-f, --from` | text |  | source unit, base or encoding |

Binary gigabytes to decimal megabytes.

```shell
1GiB | convert MB
```

```text
1073.741824
```

A number in hexadecimal.

```shell
255 | convert hex
```

```text
0xff
```

Text in base64.

```shell
"ahoj" | convert base64
```

```text
YWhvag==
```

**Errors:** `SH_ARGS`, `SH_TYPE`

**Related:** [`from`](#from), [`to`](#to), [`describe`](#describe)


### `describe`

type of a value

Returns the type of a value (text, number, size, table...).

```shell
describe
```

Full name `value-type` · input: any value · output: text

The type of a size.

```shell
1MB | describe
```

```text
size
```

**Related:** [`to`](#to)


### `now`

the current time

The current time (a timestamp); the difference of two times is a duration.

```shell
now
```

Full name `time-now` · input: nothing · output: any value

Now.

```shell
now
```

**Related:** [`sleep`](#sleep)


### `sleep`

waits

Waits for the given duration (`500ms`, `2s`); Ctrl-C interrupts it.

```shell
sleep time
```

Full name `time-sleep` · input: nothing · output: nothing

| Argument | Type | Default | Description |
|---|---|---|---|
| `time` | duration |  | duration |

A short wait.

```shell
sleep 10ms
```

**Errors:** `SH_INTERRUPTED`, `SH_TYPE`

**Related:** [`now`](#now), [`tasks`](#tasks)


## Tasks

### `tasks`

task overview (everything in the background): tasks, tasks cancel|pause|resume|log <id>

An overview of all system tasks you may see: shell tasks (`async`, `bg`), services, applications, FS maintenance and unknown processes. Without an action it returns a table; `cancel`, `pause`, `resume` control a task, `log` returns the last lines of its output. In the terminal window the same is the Ctrl-T panel.

```shell
tasks [action] [id] [--lines hodnota]
```

Full name `task-list` · input: nothing · output: table (id, name, state, owner, who, agent, progress, eta, cpu, memory, started, ended)

| Argument | Type | Default | Description |
|---|---|---|---|
| `[action]` | text | výpis | cancel, pause, resume or log (default: list) |
| `[id]` | number |  | task number |
| `-n, --lines` | number | 20 | how many output lines (tasks log) |

Services only.

```shell
tasks | where owner == "service"
```

The last 50 output lines of task 12.

```shell
tasks log 12 -n 50
```

**Needs:** the task overview (taskd); others' tasks only with a right

**Errors:** `NOT_FOUND`, `NOT_SUPPORTED`, `ACCESS_DENIED`

**Related:** [`jobs`](#jobs), [`cancel`](#cancel), [`wait`](#wait)


### `jobs`

background jobs

This session's background jobs with their state. `&` at the end of a line starts a command in the background (`cp big.img /backup &`), Ctrl+Z moves a running command there. The job keeps running and the session takes further commands; the end of the job is announced by a line in its block. `fg N` brings it back to the foreground, `kill N` ends it. What a background job changes in variables and the folder does not show up in the session.

```shell
jobs
```

Full name `job-list` · input: nothing · output: table (id, state, command)

How many jobs run in the background.

```shell
jobs | count
```

```text
0
```

**Related:** [`fg`](#fg), [`kill`](#kill), [`tasks`](#tasks)


### `fg`

brings a job from the background to the foreground (Ctrl+C interrupts it, Ctrl+Z sends it back)

Brings a job from the background (Ctrl+Z or `&`) to the foreground: the terminal waits for it like for a normal command, Ctrl+C interrupts it and Ctrl+Z sends it back. Without a number it takes the last job. Works in the terminal, not in a script.

```shell
fg [id]
```

Full name `job-foreground` · input: nothing · output: nothing

| Argument | Type | Default | Description |
|---|---|---|---|
| `[id]` | number | poslední | job number from `jobs` (default: the last one) |

Job 1 back to the foreground.

```shell
fg 1
```

**Errors:** `SH_ARGS`

**Related:** [`jobs`](#jobs), [`kill`](#kill)


### `wait`

waits for a job and returns its result

Waits for a job (the last one without a number) and returns its result or its error.

```shell
wait [id]
```

Full name `job-wait` · input: nothing · output: any value

| Argument | Type | Default | Description |
|---|---|---|---|
| `[id]` | number | poslední | job number (default: the last one) |

Waits for job 2 from `jobs`.

```shell
wait 2
```

**Errors:** `NOT_FOUND`, `SH_INTERRUPTED`

**Related:** [`jobs`](#jobs), [`kill`](#kill), [`cancel`](#cancel)


### `kill`

ends a job

Ends a background job by its number from `jobs`.

```shell
kill id
```

Full name `job-kill` · input: nothing · output: nothing

| Argument | Type | Default | Description |
|---|---|---|---|
| `id` | number |  | job number |

Ends job 3.

```shell
kill 3
```

**Errors:** `NOT_FOUND`

**Related:** [`jobs`](#jobs), [`cancel`](#cancel)


### `cancel`

cancels an async task (or a list of tasks)

Cancels an `async` task (or a list of tasks): the task is interrupted and ends with `SH_INTERRUPTED`.

```shell
cancel task
```

Full name `task-cancel` · input: nothing · output: nothing

| Argument | Type | Default | Description |
|---|---|---|---|
| `task` | value |  | a task or a list of tasks |

Cancels a long task.

```shell
let u = async { sleep 1h }; cancel $u
```

**Errors:** `SH_TYPE`

**Related:** [`tasks`](#tasks), [`wait`](#wait), [`kill`](#kill)


## Session and safety

### `undo`

reverts the changes of the last command

Reverts the file changes of the last command (an FS snapshot from before it). Works for `save`, `rm`, `mv`, `cp`, `mkdir` and scripts.

```shell
undo
```

Full name `command-undo` · input: nothing · output: any value

Then `undo` brings the file back.

```shell
rm a.txt
```

**Errors:** `SH_NO_UNDO`, `SH_CONFLICT`

**Related:** [`preview`](#preview), [`history`](#history)


### `preview`

runs a block as a dry run, shows the changes and applies them after confirmation

Runs a block as a dry run over a copy of the files, shows the list of changes and applies them after confirmation (`-y` without asking). If the files changed elsewhere meanwhile, it refuses to apply.

```shell
preview body [-y]
```

Full name `command-preview` · input: nothing · output: any value

| Argument | Type | Default | Description |
|---|---|---|---|
| `body` | block |  | a block of commands `{ ... }` |
| `-y, --yes` | switch |  | apply without asking |

A preview of a deletion, applied.

```shell
preview -y { rm a.txt }
```

**Needs:** a file system with clones (fsd)

**Errors:** `SH_REJECTED`, `SH_CONFLICT`

**Related:** [`undo`](#undo), [`rm`](#rm)


### `history`

command history with context

The session's command history as a table: line, folder, result, duration and time.

```shell
history
```

Full name `history-list` · input: nothing · output: table (line, cwd, ok, duration, time)

Commands that failed.

```shell
history | where ok == false | select line
```

**Related:** [`undo`](#undo)


### `run`

runs a script (checks its manifest needs)

Runs a script from a file with the `.mos` extension (MagnumOpus Shell). The script header (`lang 1`, `needs`) says what the script needs; the shell checks it and asks before running. `run` refuses other files (logs, text).

```shell
run path
```

Full name `script-run` · input: nothing · output: any value

| Argument | Type | Default | Description |
|---|---|---|---|
| `path` | path |  | script file (.mos) |

Saves and runs a script.

```shell
echo "echo ze skriptu" | save /tmp/s.mos; run /tmp/s.mos
```

```text
ze skriptu
```

**Needs:** what the script declares in `needs`

**Errors:** `NOT_FOUND`, `SH_NEEDS`, `SH_PARSE`

**Related:** [`error`](#error)


### `error`

raises an error

Raises an error with a message (and optionally a code); a script ends unless `try` catches it.

```shell
error message [--code hodnota]
```

Full name `error-raise` · input: nothing · output: nothing

| Argument | Type | Default | Description |
|---|---|---|---|
| `message` | text |  | error message |
| `--code` | text | SH_USER | error code |

`try` catches the error as a record (ok, failed, value, error).

```shell
(try (error "selhalo")).failed
```

```text
true
```

**Errors:** `SH_USER`

**Related:** [`explain`](#explain)


### `exit`

ends the session

Ends the shell session (the terminal window closes the tab).

```shell
exit
```

Full name `session-exit` · input: nothing · output: nothing

End of the session.

```shell
exit
```


## Help

### `help`

help for a command

Without an argument an overview of commands by group, with a command its full help: usage, arguments, pipeline, examples, errors and related commands. Ctrl+C interrupts a running command; what you type meanwhile waits in a queue and runs after it (Esc clears the queue).

```shell
help [command]
```

Full name `help-show` · input: nothing · output: text

| Argument | Type | Default | Description |
|---|---|---|---|
| `[command]` | text |  | command |

Help for `ls`.

```shell
help ls
```

**Errors:** `SH_UNKNOWN`

**Related:** [`explain`](#explain)


### `explain`

explains an error code or a log message

Explains an error code (`NOT_FOUND`, `SH_PARSE`, `-8`) or a log message: what it means and what to do.

```shell
explain code
```

Full name `error-explain` · input: any value · output: text

| Argument | Type | Default | Description |
|---|---|---|---|
| `code` | text |  | code or message |

What a type error means.

```shell
explain SH_TYPE
```

**Errors:** `SH_UNKNOWN`, `SH_ARGS`

**Related:** [`help`](#help), [`error`](#error)


## System

### `audio`

sound: tone [Hz], play file.wav, volume [%], mute, unmute, status, devices, device N

Controls sound: a test tone, playing a WAV file, volume, mute, status and device selection.

```shell
audio action ... [--duration hodnota] [--level hodnota]
```

Full name `audio-control` · input: nothing · output: any value

| Argument | Type | Default | Description |
|---|---|---|---|
| `action` | text |  | tone, play, volume, mute, unmute, status, devices or device |
| `...` | value |  | action value (Hz, file, %, device number) |
| `-d, --duration` | duration | 1s | tone length |
| `-l, --level` | number | 50 | tone level in % |

A short 440 Hz tone.

```shell
audio tone 440 -d 500ms
```

**Needs:** the sound service (audiod)

**Errors:** `NOT_FOUND`, `NOT_SUPPORTED`


### `vzhled`

appearance: svetly (light), tmavy (dark), auto (by the sun); kontrast ano|ne, pohyb ano|ne; status without arguments

Switches the appearance of the whole environment: light, dark or automatic by sunrise and sunset in the time zone. `kontrast ano` turns on high contrast (Accessibility, overrides light and dark), `pohyb ano` reduces motion (switch without animation). The choice is stored with the account and applies at once and after the next login. With `--system` it changes the login screen choice for everyone; that is an administration action confirmed by a person with the administration role in a system dialog.

```shell
vzhled [what] [value] [-s]
```

Full name `system-appearance` · input: nothing · output: table (schema, kontrast, omezit_pohyb)

| Argument | Type | Default | Description |
|---|---|---|---|
| `[what]` | text |  | svetly, tmavy, auto, kontrast or pohyb |
| `[value]` | text |  | ano or ne (for kontrast and pohyb) |
| `-s, --system` | switch |  | the system choice (login screen) |

Dark appearance.

```shell
vzhled tmavy
```

Light by day, dark at night.

```shell
vzhled auto
```

High contrast.

```shell
vzhled kontrast ano
```

**Needs:** a user session (with `--system` an administrator's confirmation)

**Errors:** `SH_ARGS`, `ACCESS_DENIED`, `CANCELED`, `NOT_SUPPORTED`

**Related:** [`audio`](#audio)


### `povoleni`

app permissions: list without arguments, `povoleni odvolat <app> [mikrofon]` revokes

Without arguments lists the user's stored decisions about app permissions (the microphone today). `povoleni odvolat <app>` deletes the decision: the app asks again next time.

```shell
povoleni
```

Full name `system-permissions` · input: nothing · output: nothing

The app asks for the microphone again.

```shell
povoleni odvolat org.libsdl.testaudio
```

**Needs:** a signed-in session (decisions belong to the session's user)

**Errors:** `SH_ARGS`, `NOT_FOUND`, `ACCESS_DENIED`, `NOT_SUPPORTED`

**Related:** [`audio`](#audio)


### `loglevel`

log level: list without arguments, `loglevel <module|process> <error|warn|info|debug|trace|default>`

Without arguments lists the log levels; with a module or process and a level it changes it (until restart).

```shell
loglevel
```

Full name `system-log-level` · input: nothing · output: nothing

Detailed log of the fsd service.

```shell
loglevel fsd debug
```

**Needs:** the DIAG right

**Errors:** `SH_ARGS`, `ACCESS_DENIED`

**Related:** [`report`](#report)


### `report`

a bundle for developers: logs, crash records and machine information in one file (prints its path and content)

Packs logs, crash records and machine information into one file for developers and prints its path and an overview of its content.

```shell
report
```

Full name `system-report` · input: nothing · output: nothing

Creates the bundle.

```shell
report
```

**Needs:** the DIAG right

**Errors:** `ACCESS_DENIED`

**Related:** [`loglevel`](#loglevel)


### `install`

installs the system from the live USB onto a disk (a step-by-step wizard; with --odpovedi without questions)

A step-by-step installation wizard like the graphical installer: language and keyboard, computer check, where to install, preview of disk changes, account, security, summary and progress. It never overwrites foreign partitions; erasing a whole disk requires typing the disk name. With `--odpovedi <file>` it runs without questions.

```shell
install [--odpovedi hodnota]
```

Full name `system-install` · input: nothing · output: nothing

| Argument | Type | Default | Description |
|---|---|---|---|
| `--odpovedi` | text |  | answers file (installation without questions) |

Starts the wizard.

```shell
install
```

**Needs:** the live USB (installer)

**Errors:** `SH_REJECTED`, `SH_USER`, `NOT_SUPPORTED`

**Related:** [`report`](#report)


### `shutdown`

shuts the computer down cleanly (ends services, then ACPI S5)

Shuts the computer down cleanly: asks (without `-y`), ends the services and saves data, then ACPI S5.

```shell
shutdown [-y]
```

Full name `system-shutdown` · input: nothing · output: nothing

| Argument | Type | Default | Description |
|---|---|---|---|
| `-y, --yes` | switch |  | shut down without asking |

Shutdown without asking.

```shell
shutdown -y
```

**Needs:** the power right

**Errors:** `SH_REJECTED`, `ACCESS_DENIED`

**Related:** [`reboot`](#reboot)


### `reboot`

restarts the computer cleanly

Restarts the computer cleanly (like `shutdown`, then a restart).

```shell
reboot [-y]
```

Full name `system-reboot` · input: nothing · output: nothing

| Argument | Type | Default | Description |
|---|---|---|---|
| `-y, --yes` | switch |  | restart without asking |

Restart without asking.

```shell
reboot -y
```

**Needs:** the power right

**Errors:** `SH_REJECTED`, `ACCESS_DENIED`

**Related:** [`shutdown`](#shutdown)

