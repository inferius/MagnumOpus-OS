# Design: the shell language (for decision)

> Status: **agreed** - [two notations](#decision-one-language-two-notations-b-and-c)
> and [notation decisions](#notation-decisions). The research and variants
> chapters below show what was chosen from. So far only these points are
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
4. **Errors are values:** a code, a message and a location. An unhandled
   error propagates by itself and stops the script (like `set -e`, but
   reliably); `try {...} else {...}` handles it. Interactively it is printed with an
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

## Decision: one language, two notations (B and C)

> **Agreed:** the language has **two notations** - command notation B (the
> default for interactive typing) and expression notation C close to Rust
> (for scripts and logic). Both are **notations of the same language**, not
> two languages. This must be guarded strictly.

```text
ls ~/Doc -r | where size > 100MB                          # B
ls("~/Doc", recursive: true).where(f => f.size > 100MB)   # C, the same
ls ~/Doc -r | where (f.size > 100MB && f.ext == "pdf")    # mixing: an expression in ( ) is C
```

**Rules that keep it together** (checked by CI, not just agreed):

1. **One tree, one semantics.** Both parsers produce the same expression
   tree. Types, evaluation, pipes, errors, the permission manifest,
   completion and help exist only once, on top of the tree. A notation may
   not have behaviour of its own.
2. **Every construct in both notations.** A new language feature is
   accepted only with a form in both B and C, unless a form makes no sense
   in one of them; such an exception is written down explicitly in the
   exceptions table in this chapter.
3. **Round trip is a test.** The formatter can do `fmt --style b|c`. For
   every example in the documentation and the test suite, tests check that
   `B -> tree -> C -> tree` gives the same tree and the same result, and the
   same in reverse.
4. **The documentation shows both forms** (generated by the formatter, not
   written twice by hand).
5. **Mixing is allowed** only at a clear boundary: an expression in `( )`
   inside a command is C; a command inside an expression is called as a
   function.
6. **The language version (`lang 1`) applies to both notations** at once.

**Exceptions table** (constructs in one notation only): empty so far.

## Notation decisions

| # | Question | Decision |
|---|---|---|
| 1 | basic style | **B and C as two notations of one language** |
| 2 | command names | **both**: a short name (`ls`, `cp`) and a full `verb-noun` name (`file-list`) as an alias; help shows both |
| 3 | variables | **`$` only in command arguments** (where a word would otherwise be text); no `$` in expressions, `where` conditions and notation C. A bare word in an argument named like a variable triggers a hint "did you mean `$x`?" |
| 4 | function (closure) | **`x => expression`** and `(a, b) => expression`, a body with several commands `x => { ... }`, in both notations; named `fn name(x) { }`. Not `\|x\|` - `\|` is the pipe in a shell |
| 5 | errors | **an unhandled error propagates by itself** (like an exception) and stops the script; **an error is a value** with a code, message, location and cause. Handling `try { } else e => ...`, fallback `cmd \|\| other`, the result as a value `let r = try cmd`. `?` only optionally in functions in C |
| 6 | Czech | **English names**, Czech help and error messages |
| 7 | script without `lang` | **latest version + a warning** |

## Rules of notation B

- **Command mode:** `command arg -p value --long`. A word is text, `$x` is
  a variable's value, and the shell expands `*.txt` against the FS (only
  when unquoted). Arguments are typed from the program's manifest, so an
  error like "expected a path, got a number" comes before the program runs.
- **Expression mode:** after `let`, `if`, `for`, `while`, `return`, in a
  `where` condition and inside `( )`. It has numbers with units (`100MB`,
  `1y`, `30s`), comparisons, `&&` `||` `!`, strings in `"..."` with
  `{expression}` inside, lists `[1, 2]` and records `{name: "a", size: 1KB}`.
  Variables have no `$`.
- **Pipe:** `|` passes records (or bytes, according to the manifest). Common
  operations are short words: `where`, `select`, `sort`, `group`, `first`,
  `count`, `each f => ...`. Conversions are explicit: `from json`,
  `to text`, `lines`.
- **Errors:** an unhandled error stops the script with an explanation.
  `try { rm $f } else e => log "failed: {e.message}"` handles it,
  `rm $f || echo "never mind"` uses a fallback, and `let r = try rm $f`
  returns the result as a value (`r.failed`, `r.error.code`).
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
    image resize $f.path --width 1600 -o ~/Photos/small/{f.name}
}
```

- **Safe defaults:** a command that changes files gets a lazy snapshot
  (undo). A bulk change over N files runs through "show what will happen"
  ([terminal](architecture.md#shell-and-smart-terminal-proposal)).
