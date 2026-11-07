# Design: the shell language (for decision)

> Status: **a proposal for discussion**. So far only these points are
> agreed: it is a new typed language, an error is a value (`?` as in Rust),
> a script carries a permission manifest, and POSIX `sh` is separate
> ([decisions 6a-6d](architecture.md#decisions-for-milestones-6a-6d-agreed)).
> This page covers the syntax and behaviour still to be chosen.

## What to learn from others

| Shell | What works | What people complain about |
|---|---|---|
| **Nushell** | tables and records in pipes, `where`/`sort-by` without `grep`/`awk`, readable errors pointing into the command | frequent breaking changes between versions; job control and `&&`/`\|\|` were missing for a long time; its own `ls` etc. behave differently from system tools; weaker error and Ctrl-C handling; "more a programming language than a shell" |
| **PowerShell** | objects in pipes, uniform parameters, help | verbose names (`Get-ChildItem`); objects live only inside .NET; **the pipe corrupts binary data** (a file downloaded through `>` ends up twice the size); hidden type conversions; hard-to-debug pipelines |
| **Oils (YSH)** | an upgrade path from `sh` to a language with data; two modes - command and expression; structured data "outside" (JSON/TSV between processes) | two languages in one, slow progress |
| **Elvish** | values (lists, maps, functions) in pipes; a non-zero exit code = an exception | structured data only between built-ins; external programs get bytes |
| **Murex** | typed data in pipes, `try/catch`, errors with line numbers | small community |
| **fish** | no word splitting, sane quoting, suggestions from history, syntax highlighting, help | text only, no structures |

Sources: [The case for Nushell (HN)](https://news.ycombinator.com/item?id=37313733),
[I want to like Nushell](https://experimentalworks.net/posts/2024-02-24-i-want-to-like-nushell/),
[Ask HN: alternative shells as daily drivers](https://news.ycombinator.com/item?id=34722208),
[Oils: exterior-first](https://www.oilshell.org/blog/2023/06/ysh-design.html),
[Oils: why objects](https://www.oilshell.org/blog/2024/12/objects.html),
[PowerShell and binary data](https://brianreiter.org/tag/pipeline/),
[Elvish: unique semantics](https://elv.sh/learn/unique-semantics.html),
[Murex](https://github.com/lmorg/murex).

**How we differ:** we own the OS. Programs declare their input and output
in a manifest, and a pipe carries binary records with a schema
([terminal](architecture.md#terminal-lazy-undo-and-sessions)). Structured
data is therefore not a privilege of built-ins (unlike Nushell and Elvish),
and nothing has to be parsed from JSON. That lets us be structured "outside",
like Oils, without a text step in between.

## Principles (what follows)

1. **Fast to type interactively, readable as scripts.** Short names (`ls`,
   `cp`, `where`), not `Get-ChildItem`.
2. **Two modes, like Oils and PowerShell:** command mode (words are text,
   `ls -l /tmp`) and expression mode (`let`, `if`, comparisons, numbers with
   units). The mode boundary is visible at a glance.
3. **Nothing changes silently.** No word splitting, no `*` expansion inside
   quotes, no hidden type conversions. **A byte stream is a different type
   from a record stream**, and converting between them is always explicit
   (`from json`, `to text`). PowerShell's binary-data bug cannot happen.
4. **Errors are values:** a code, a message and a location. `?` passes the
   error up; `try {...} else {...}` handles it. An unhandled error stops the
   script (like `set -e`, but reliably). Interactively it is printed with an
   explanation from [errors.md](errors.md).
5. **What you expect from a shell is there from the start:** `&&` and
   `||`, job control (background, switching, Ctrl-C always works) and
   redirection.
6. **Stability:** a script declares the language version (`lang 1`). A
   breaking change means a new version; the old one keeps running. This is
   the lesson from Nushell.
7. **Built-ins do not imitate foreign tools.** They are our programs with a
   manifest: `ls` returns file records. POSIX tools get a separate `sh` and
   the VT100 adapter.

## Syntax variants

The same task in each variant: large files older than a year, sorted, then
deleting them with confirmation.

**A - record pipes (like Nushell)**

```text
ls ~/Documents -r | where size > 100MB and mtime < now - 1y | sort-by size -d
ls ~/Documents -r | where size > 100MB | each { |f| rm $f.path }
```

**B - command + expression mode (recommended)**

```text
ls ~/Documents -r | where size > 100MB && mtime < now - 1y | sort size desc
let big = ls ~/Documents -r | where size > 100MB
for f in $big { rm $f.path? }          # ? = an error stops the script
if (df /).free < 5GB { notify "Running out of space" }
```

**C - closer to Rust**

```text
ls("~/Documents", recursive: true).filter(|f| f.size > 100.mb && f.mtime < now() - 1.year).sort_by(.size).rev()
```

| | A Nushell | B two modes | C Rust |
|---|---|---|---|
| interactive typing | good | **good** | verbose |
| scripts, logic | good | **good** (expressions as in a programming language) | best |
| learning for bash users | medium | **easiest** (commands look as before) | hard |
| text / expression boundary | unclear (`$x` vs `x`) | **clear** (`let`, `if`, `(...)` = expression) | expressions only |
| completion by type | yes | yes | yes |

## Recommendation: B with these rules

- **Command mode:** `command arg -p value --long`. A word is text, `$x` is
  a variable's value, and the shell expands `*.txt` against the FS (only
  when unquoted). Arguments are typed from the program's manifest, so an
  error like "expected a path, got a number" comes before the program runs.
- **Expression mode:** after `let`, `if`, `for`, `while`, `return` and
  inside `( )`. It has numbers with units (`100MB`, `1y`, `30s`),
  comparisons, `&&` `||` `!`, strings in `"..."` with `{expression}`
  inside, lists `[1, 2]` and records `{name: "a", size: 1KB}`.
- **Pipe:** `|` passes records (or bytes, according to the manifest). Common
  operations are short words: `where`, `select`, `sort`, `group`, `first`,
  `count`, `each { |x| ... }`. Conversions are explicit: `from json`,
  `to text`, `lines`.
- **Errors:** `command?` passes the error up, and
  `try { } else |e| { }` handles it, with `$e.code` and `$e.message`
  available. `&&`/`||` follow success.
- **Script manifest:** a header the shell checks before running the script;
  it hands out capabilities according to it:

```text
lang 1
needs {
    read: ["~/Photos"]
    write: ["~/Photos/small"]
    net: none
}
for f in (ls ~/Photos | where ext == "jpg") {
    image resize $f.path --width 1600 -o ~/Photos/small/{f.name}?
}
```

- **Safe defaults:** a command that changes files gets a lazy snapshot
  (undo). A bulk change over N files runs through "show what will happen"
  ([terminal](architecture.md#shell-and-smart-terminal-proposal)).

## Open questions to decide

| # | Question | Variants | Recommendation |
|---|---|---|---|
| 1 | basic style | A / **B** / C | **B** |
| 2 | command names | short (`ls`, `cp`) / `verb-noun` (`file-list`) / both (short alias) | short, the full name in help |
| 3 | variables in a command | `$x` / no sigil (only in expressions) | `$x` in commands, no `$` in expressions |
| 4 | function (closure) syntax | `{ \|x\| ... }` / `fn(x) { ... }` / `x => ...` | `{ \|x\| ... }` (short in a pipe), `fn name(x) { }` for named functions |
| 5 | errors | values + `?` / exceptions (Elvish) | values + `?` (agreed) |
| 6 | Czech in commands | English names only / localised aliases | English names, Czech help and error messages |
| 7 | no `lang` in a script | error / latest version | latest version + a warning |
