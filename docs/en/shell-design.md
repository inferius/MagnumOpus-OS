# Shell language and smart terminal

> Status: **agreed and implemented (milestone 6b)** - [two notations](#decision-one-language-two-notations-b-and-c),
> [notation decisions](#notation-decisions) and [grammar decisions](#grammar-decisions).
> The language is described in [Grammar](#language-grammar-lang-1) and
> [Semantics](#semantics), the terminal in [Terminal and session service](#terminal-and-session-service).
> The research and variants chapters show what was chosen from.
>
> Code: `libs/shlang` (language, `no_std` + `alloc`, host tests),
> `libs/term` (protocol, sessions, VT100, screen), `user/termd` (broker), `user/shsess` (session process).

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

```mos
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

```mos
ls ~/Doc -r | where size > 100MB                          # B
ls("~/Doc", recursive: true).where(f => f.size > 100MB)   # C, the same
ls ~/Doc -r | where (size > 100MB && ext == "pdf")      # mixing: an expression in ( ) is C
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

**Exceptions table** (constructs in one notation only): none. Notation B
shorthands have a full form with the same tree in C, see
[Notation B shorthands](#notation-b-shorthands-and-their-full-form).

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

```mos
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

- **Safe defaults:** a command that changes files gets a draft in a
  file system clone and undo. A bulk change over N files runs through "show what will happen"
  ([terminal](architecture.md#shell-and-smart-terminal-proposal)).

## Grammar decisions

| # | Question | Decision |
|---|---|---|
| 1 | boundary between command and expression mode in B | **by variable scope**: a bare word at the start of a pipeline part (statement, `( )`, closure body) is a command unless it is a variable or a keyword. Otherwise it is an expression. `(x + 1)` is an expression when `x` is a variable; `(ls ~)` is a nested command |
| 2 | the `where` condition | a bare name is a **variable when the parser sees it in scope**, otherwise a field of the row `it`. When a variable has the name of a known input field (schema from the previous command's signature), it is an **error** with the hint `it.size`. A later `let size = 5` can therefore never silently change what a filter means |
| 3 | redirection | `>`, `>>` and `<` work **only in command mode** and are shorthand: `cmd > f` is `cmd \| save f`, `>>` is `save --append`, `cmd < f` is `open f \| cmd`. In an expression (the `where` condition, `if`, `( )`) `>` is always a comparison. Records go to a file only with an explicit `to text`/`to json` |
| 4 | choosing the notation | **per statement**: a word directly followed by `(` (`ls("~")`) or `.command(` is the C form, otherwise B. The header `lang 1 c` forces notation C for the whole file (a B command is then an error) |

## Language grammar (lang 1)

<figure class="diagram">
<svg viewBox="0 0 760 270" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="One tree for both notations">
<defs><marker id="se1" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="box" x="10" y="20" width="230" height="56" rx="6"/><text class="t b" x="125" y="42" text-anchor="middle">notation B (command)</text><text class="mono t" x="125" y="62" text-anchor="middle">ls ~ -r | where size &gt; 1MB</text>
<rect class="box" x="10" y="110" width="230" height="56" rx="6"/><text class="t b" x="125" y="132" text-anchor="middle">notation C (expression)</text><text class="mono t" x="125" y="152" text-anchor="middle">ls("~", recursive: true)...</text>
<path class="ln" d="M240 48 L318 82" marker-end="url(#se1)"/>
<path class="ln" d="M240 138 L318 104" marker-end="url(#se1)"/>
<rect class="acc" x="320" y="55" width="150" height="76" rx="6"/><text class="t b" x="395" y="82" text-anchor="middle">shared tree</text><text class="m" x="395" y="100" text-anchor="middle">ast.rs, variable</text><text class="m" x="395" y="116" text-anchor="middle">scope</text>
<path class="ln" d="M470 70 L548 34" marker-end="url(#se1)"/>
<path class="ln" d="M470 93 L548 93" marker-end="url(#se1)"/>
<path class="ln" d="M470 116 L548 152" marker-end="url(#se1)"/>
<rect class="ok" x="550" y="10" width="200" height="46" rx="6"/><text class="t b" x="650" y="30" text-anchor="middle">formatter</text><text class="m" x="650" y="47" text-anchor="middle">fmt --style b | c</text>
<rect class="ok" x="550" y="70" width="200" height="46" rx="6"/><text class="t b" x="650" y="90" text-anchor="middle">interpreter</text><text class="m" x="650" y="107" text-anchor="middle">types, pipes, errors</text>
<rect class="ok" x="550" y="130" width="200" height="46" rx="6"/><text class="t b" x="650" y="150" text-anchor="middle">completion, help</text><text class="m" x="650" y="167" text-anchor="middle">command signatures</text>
<path class="ln-acc dash" d="M650 56 C 650 230, 125 230, 125 168" marker-end="url(#se1)"/>
<text class="m" x="10" y="250">Conversion test: B -&gt; tree -&gt; C -&gt; tree gives the same tree and the same result (and back). It is a required gate in cargo xtask test.</text>
</svg>
<figcaption>Both parsers build the same tree; everything else works only with the tree.</figcaption>
</figure>

### Lexical elements

- **Comment:** `#` to the end of the line. A comment on its own line is kept
  by the formatter.
- **Numbers:** `42`, `-5`, `3.14`, `1_000_000`. **With a unit** (no space):

  | Kind | Units | Value |
  |---|---|---|
  | size | `B`, `KB`, `MB`, `GB`, `TB` (×1000), `KiB`, `MiB`, `GiB`, `TiB` (×1024) | bytes |
  | duration | `ns`, `us`, `ms`, `s`, `min`, `h`, `d`, `w`, `y` (365 days) | nanoseconds |

- **Strings:** `"..."` with the escapes `\n \t \r \0 \\ \" \{ \}` and
  embedded expressions `{expr}`; `'...'` is literal text without escapes and
  without `{}`.
- **Names:** letters (including accented ones), digits and `_`. Command
  names in B may contain `-` (`file-list`); C uses `_` (`file_list(...)`)
  and the parser maps it back when such a command exists.
- **Keywords:** `let fn if else for in while return break continue try bg
  true false null lang needs`.

### Grammar (EBNF)

```text
script      = [ "lang" NUMBER [ "b" | "c" ] END ] [ needs END ] { stmt END }
needs       = "needs" "{" { ("read" | "write") ":" path_list | "net" ":" ("none" | path_list) } "}"
stmt        = "let" NAME "=" chain
            | NAME "=" chain                           (* NAME is a variable *)
            | "fn" NAME "(" [ NAME { "," NAME } ] ")" block
            | "return" [ chain ] | "break" | "continue"
            | chain
block       = "{" { stmt END } "}"

(* notation B *)
chain       = and_chain { "||" and_chain } [ "&" ]
and_chain   = pipeline { "&&" pipeline }
pipeline    = part { "|" call_B } [ (">" | ">>") arg | "<" arg ]
part        = call_B                                   (* a bare word that is not a variable *)
            | expr_without_logic                       (* otherwise *)
call_B      = COMMAND { arg | flag } | COMMAND "(" args_C ")"
flag        = "-" LETTERS | "--" NAME [ "=" arg ] | "--"
arg         = "$" NAME { "." NAME } | STRING | "(" chain ")" { suffix }
            | list | record | block | closure | WORD
WORD        = text, number with unit, true/false/null, pattern (*.txt) or word with {expr}

(* notation C: chain = expr *)
expr        = closure | expr "||" expr | expr "&&" expr | comparison
comparison  = sum [ ("==" | "!=" | "<" | "<=" | ">" | ">=") sum ]
sum         = product { ("+" | "-") product }
product     = unary { ("*" | "/" | "%") unary }
unary       = ("!" | "-") unary | base { suffix }
suffix      = "." NAME | "." NAME "(" args_C ")" | "[" expr "]" | "?"
base        = NUMBER | STRING | NAME | NAME "(" args_C ")" | "glob" "(" STRING ")"
            | "(" expr ")" | list | record | block
            | "if" expr block [ "else" ( block | if ) ] | "for" NAME "in" expr block
            | "while" expr block | "try" ( block | part ) [ "else" ( closure | block ) ]
            | "bg" block
args_C      = [ arg_C { "," arg_C } ]
arg_C       = [ NAME ":" ] expr
closure     = ( NAME | "(" [ NAME { "," NAME } ] ")" ) "=>" ( block | body )
list        = "[" [ expr { "," expr } ] "]"
record      = "{" [ key ":" expr { "," key ":" expr } ] "}"
```

Expressions (arithmetic, comparisons, closures, `if`, `try`, ...) are **the
same in both notations**. The notations differ only in how a command and a
pipe are written:

| Construct | Notation B | Notation C | Tree |
|---|---|---|---|
| command | `ls ~/Doc` | `ls("~/Doc")` | `Call` |
| flag | `-r`, `--recursive` | `recursive: true` | named argument `recursive` |
| flag with a value | `--width 1600`, `--width=1600` | `width: 1600` | named argument |
| variable in an argument | `$f.path` | `f.path` | `Field(Var)` |
| file pattern | `*.txt` | `glob("*.txt")` | `Glob` |
| text with an expression | `~/out/{f.name}` | `"~/out/{f.name}"` | `Interp` |
| pipe | `a \| b x` | `a().b(x)` | `Pipe` |
| condition | `where size > 1MB` | `.where(it => it.size > 1MB)` | `Closure(it)` |
| background | `cmd &` | `bg { cmd() }` | `Bg` |
| save to a file | `cmd > f`, `cmd >> f` | `.save("f")`, `.save("f", append: true)` | `Pipe(..., save)` |
| program by path, or a name with `-` and no signature | `./run.sh x`, `my-tool a` | `exec("./run.sh", "x")`, `exec("my-tool", "a")` | `Call` |

### Mode boundary in notation B

- **At the start of a pipeline part** (a line, `let x =`, the contents of
  `( )`, a closure body): a bare word that is not a variable or a keyword is
  a **command**. Otherwise it is an **expression**. A word directly
  followed by `(` is a call in the C form.
- **After `|`** there is always a command.
- **Command arguments:** a bare word is text. It can also be a number with a
  unit or `true`/`false`/`null` (`first 5` passes the number 5, not text).
  `$x` is the value of a variable, `( )` a nested pipeline or expression,
  `[ ]` a list, `{ }` a record or a block. Text that looks like a number
  goes in quotes (`"5"`).
- An **expression** follows `let`, is in the `if`/`while` condition, after
  `for ... in`, in the `where` condition and in an `{embedded expression}`.
  In an expression a command is called as a function (`df("/")`) or in
  parentheses (`(df /)`).
- `$` belongs **only in command arguments**. In an expression, `$x` is an
  error with the hint "write just x". A bare word in an argument that is
  named like a variable gives the warning "did you mean `$x`?".

### Notation B shorthands and their full form

The shorthands are not exceptions: each has a full form that exists in both
notations. The formatter prints the shorthand in B and the full form in C.

| B shorthand | Full form | Note |
|---|---|---|
| `cmd &` | `bg { cmd }` | background job |
| `cmd > f`, `cmd >> f` | `cmd \| save f`, `cmd \| save f --append` | `save` accepts only bytes or text |
| `cmd < f` | `open f \| cmd` | |
| `where size > 1MB` | `where it => it.size > 1MB` | `it` is the row |
| `-r` | `--recursive` | short flag from the signature |
| `-` + digit | negative number | `-5` is not a flag |
| `--` | (nothing) | further words starting with `-` are text |

### The `where` condition

```shell-b
let limit = 1MB
ls | where size > limit && ext == "pdf"
```

`size` and `ext` are not variables, so they are fields of the row. `limit`
is a variable. If a variable `size` existed, the parser would report the
error "`size` is both a variable and a row field". Then write `it.size`,
rename the variable, or use a closure with its own parameter name:

```shell-b
let size = 5
ls | where f => f.size > size
```

Inside the condition a command is called as a function (`now()`), because a
bare word is a field: `where mtime < now() - 1y`.

## Semantics

### Values and types

| Type | Example | Note |
|---|---|---|
| `int`, `float` | `42`, `1.5` | an integer is converted in arithmetic with a float (the only automatic conversion) |
| `size` | `100MB` | a size is never added to or compared with a number: `size > 100` is an error with the hint `100B` |
| `duration` | `30s`, `1h` | |
| `time` | `now()` | `time - duration` = `time`, `time - time` = `duration` |
| `text` | `"abc"` | |
| `bytes` | `open f` | a **byte stream**, a different type from text and records |
| `list`, `record` | `[1, 2]`, `{a: 1}` | a list of records is a table |
| `closure` | `x => x * 2` | captures copies of the variables it uses |
| `error` | `e` in `try ... else e => ...` | fields `code`, `message`, `hint`, `cause` |
| `null`, `nothing` | | `nothing` = the command returned nothing |

- **No hidden conversions.** Bytes, text and records are converted only
  explicitly: `from json`, `from text`, `lines`, `to text`, `to json`.
  `open data.json | where ...` is an error with the hint `from json`.
- **Equality** of different types is an error (except numbers and `null`).
  `size == 5` would otherwise silently return `false`.
- The `if`/`while`/`where` condition must be a `bool`. A value is never
  treated as "truthy".
- A field of a list of records returns a column: `(ls).name` is a list of
  names.

### Output

At the top level (a line, a loop body, `if` branches used as a statement)
the value of each statement is printed as an output block. In a closure or
function body the result is the value of the last statement. Values of
earlier statements are dropped there; use `log` for a note.

### Errors

- An unhandled error **propagates by itself** and stops the script. File
  changes the line made before the error are applied (as in other shells)
  and can be reverted with `undo`.
- `try { ... } else e => ...` handles it, `cmd || other` uses a fallback
  (the right side also runs when the left one is `false`), `let r = try cmd`
  returns the result as a record `{ok, failed, value, error}`. `x?` unwraps
  such a result or raises its error; in C `?` is optional only, because
  errors propagate anyway.
- `a && b`: an error in `a` propagates (it is not caught), `false` returns
  `false`, otherwise `b` is evaluated.
- **Ctrl-C** (`SH_INTERRUPTED`) cannot be caught: neither `try` nor `||`
  stops it.
- An error carries a code, a description, a location in the source (line,
  column, underline), a hint and a cause. `explain code` shows the
  explanation from [Error codes](errors.md) and from the
  [Shell errors chapter](#shell-errors).

### Functions, closures, jobs

- `fn name(a, b) { ... }` sees its parameters, global variables and `input`
  (the pipe input). It is called as a command (`name 1 2`, `name --b=2 --a=1`)
  or as a function (`name(1, 2)`).
- A closure captures **copies** of the variables it uses at the moment it
  is created. A later change of the variable does not affect it.
- A **background job** (`cmd &`, `bg { }`) runs cooperatively: when the
  shell is idle, or on `wait`. `jobs` lists them, `kill N` cancels a waiting
  job.
- **Recursion depth** is guarded by actual stack use. The host sets the
  budget; `shsess` has a 256 KB stack and the interpreter may use 160 KB.
  Exceeding it gives `SH_DEPTH`, not a crash.

### Language version

`lang 1` applies to both notations. A script without `lang` runs in the
newest version with a warning. An unknown version is the error `SH_LANG`.
An incompatible change = a new version; an old script keeps running in its
own.

## Built-in commands

Every command has a short name and a full `verb-noun` name (an alias). The
complete reference of all commands (description, arguments and switches,
pipeline, examples, errors, related commands) is on the page
[Shell commands](shell-commands.md). `cargo xtask docs` generates it from
the metadata in `libs/shlang/src/doc/` - `help <command>` in the terminal
shows the same data and the examples are tests (`cargo test -p shlang doc`),
so the help does not go stale.

## Examples

Each example is written in one notation. The other one (marked `*`) is
generated by the formatter during `cargo xtask docs`. The test
`cargo test -p shlang` checks the conversion both ways for every example on
this page.

Large files older than a year, sorted by size:

```shell-b
ls ~/Documents -r | where size > 100MB && mtime < now() - 1y | sort size -d
```

Deleting with error handling:

```shell-b
for f in (ls ~/tmp | where ext == "log") {
    try rm $f.path else e => log "failed: {e.message}"
}
```

Data from JSON, result to a file:

```shell-b
open people.json | from json | where age >= 18 | select name age | to json > adults.json
```

A function and a closure in notation C:

```shell-c
fn large(dir, limit) {
    return ls(dir, recursive: true).where(it => it.size > limit)
}
let total = large("~/Videos", 1GB).sum("size")
```

Fallback and the result as a value:

```shell-b
let r = try rm old.txt
if r.failed {
    echo "not deleted: {r.error.code}"
}
rm temp.txt || echo "was not there"
```

A script with a permission manifest:

```shell-b
lang 1
needs {
    read: ["~/Photos"]
    write: ["~/Photos/small"]
    net: none
}
for f in (ls ~/Photos | where ext == "jpg") {
    cp $f.path ~/Photos/small/{f.name}
}
```

## Files: draft in a clone, undo, preview and lock

> **Change from the first shell version:** changes in a dry run are no
> longer an overlay in the shell's memory but a **clone of the dataset
> "aside" directly in the file system** (`ASIDE_*` from [FS phase
> 2](fs-history.md)). Undo no longer relies on a snapshot of the whole
> volume but on a **view** (`VIEW`) of the affected datasets only. After a
> program, the changes come from the journal and the `DIFF` operation, not
> from walking the tree. Why: large writes cost no memory, a program in
> `preview` works on the same draft as built-in commands, and undo after a
> program is fast even on a large disk.

<figure class="diagram">
<svg viewBox="0 0 760 270" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Line draft in a file system clone">
<defs><marker id="sh2" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="box" x="10" y="80" width="130" height="56" rx="6"/><text class="t b" x="75" y="104" text-anchor="middle">command</text><text class="m" x="75" y="122" text-anchor="middle">rm, save, mv, program</text>
<path class="ln" d="M140 108 L188 108" marker-end="url(#sh2)"/>
<rect class="acc" x="190" y="70" width="150" height="76" rx="6"/><text class="t b" x="265" y="94" text-anchor="middle">clone aside</text><text class="m" x="265" y="112" text-anchor="middle">ASIDE_BEGIN on</text><text class="m" x="265" y="128" text-anchor="middle">the first write</text>
<path class="ln" d="M340 108 L378 108" marker-end="url(#sh2)"/>
<rect class="box" x="380" y="80" width="130" height="56" rx="6"/><text class="t b" x="445" y="104" text-anchor="middle">ASIDE_CHANGES</text><text class="m" x="445" y="122" text-anchor="middle">end of line</text>
<path class="ln" d="M510 95 L558 48" marker-end="url(#sh2)"/>
<path class="ln" d="M510 122 L558 172" marker-end="url(#sh2)"/>
<rect class="ok" x="560" y="20" width="190" height="56" rx="6"/><text class="t b" x="655" y="42" text-anchor="middle">≤ N: VIEW + APPLY</text><text class="m" x="655" y="60" text-anchor="middle">view for undo, apply</text>
<rect class="warn" x="560" y="145" width="190" height="56" rx="6"/><text class="t b" x="655" y="167" text-anchor="middle">&gt; N or preview</text><text class="m" x="655" y="185" text-anchor="middle">diff, confirm / DISCARD</text>
<path class="ln dash" d="M655 145 L655 78" marker-end="url(#sh2)"/>
<text class="m" x="10" y="236">A reading line creates no clone and costs nothing. Clones live in /tmp/.aside/&lt;session&gt;-&lt;n&gt;; nested datasets are</text>
<text class="m" x="10" y="254">boundaries there and a write into one gets its own clone. Applying is three-way: a foreign change = SH_CONFLICT, not an overwrite.</text>
</svg>
<figcaption>A line's changes go to a clone of the dataset in the file system; at the end of the line they are applied with a view for undo.</figcaption>
</figure>

- **Line draft:** the first write of a line into a dataset creates its
  clone (`ASIDE_BEGIN`, `libs/shlang/src/vfs/draft.rs`). Further writes and
  reads of that dataset go to the clone, so the line sees its own changes.
  A write into a nested dataset (a boundary in the clone) gets a clone of
  that dataset. At the end of the line the shell takes the diff from the
  file system (`ASIDE_CHANGES`). A line that only reads creates no clone
  and costs nothing extra.
- **Undo:** before applying (`ASIDE_APPLY`) the shell creates a **view**
  (`VIEW`, a cheap temporary snapshot and a journal position) of every
  dataset with changes. `undo` syncs the changed paths with the state in
  the view (reading via `SNAP_*_OF`) and works repeatedly (last 16
  commands, older views are released). The file system itself cleans up
  views after a restart.
- **Conflict:** applying is three-way against the state the clone started
  from. When another session or a program changed the same file
  meanwhile, the change is not applied and the line ends with
  `SH_CONFLICT` and the list of paths. The other changes are written and
  `undo` reverts only them.
- **"Show what will happen":** `preview { ... }` adds another layer (a
  clone of the clone), runs the block, shows a table of changes (`added`,
  `modified`, `removed`, `dir-added`, `dir-removed`, `replaced` = a file
  replaced by a directory or the other way round, `moved`) and applies it
  to the line draft only after confirmation (`-y` without asking).
  Rejecting = `ASIDE_DISCARD`, `SH_REJECTED` and nothing changes.
- **Dangerous operation lock:** a command that would change more than N
  files outside `/tmp` (default N = 20) always asks before the changes are
  applied. It is the same question as with `preview`. Without an attached
  window it is refused.
- **The `needs` manifest:** `run script` shows what the script wants
  (`read`, `write`, `net`) before it starts and, after confirmation,
  restricts it: access outside the listed paths is `SH_NEEDS`. A script gets
  at most the intersection with its caller's rights: writes only where the
  caller may write, reads where it may read, network only with network.
  Otherwise `SH_NEEDS` before it starts. A background job (`bg`, `&`)
  carries the restriction with it, even when it finishes after the script
  has ended.
- **Programs:** get the permissions from their manifest through their own
  fsd session (`CONNECT_AS` with mounted paths, `term::perm::mounts`; write
  overrides read). Outside `preview` the shell applies the line draft so
  far before the program starts, **with the same lock** as at the end of a
  line (rejecting = the program does not start), and creates views of the
  datasets the program may write to (with full rights `/`, home and the
  working directory). Afterwards one journal message tells whether
  anything changed. If not, the undo entry is dropped, so `undo` reverts
  the previous command. If so, `DIFF` returns the changed paths and `undo`
  syncs them with the view.
- **A program in `preview`:** the shell creates clones of the datasets the
  program may write to and mounts them for it **under the original names**
  (`ExecNeeds::aside`). Writes outside the clones get read-only access. The
  program thus changes only the draft: its changes are in the preview
  table and disappear after rejecting.

```shell-b
preview { rm *.tmp; echo hotovo | save ~/vysledek.txt }
undo
```

⚠️ **Limits:**

- A clone cannot be created where the file system has no writable
  `/tmp`. The write then ends with an error and a hint (`NOT_SUPPORTED`).
- Undo of a program with full rights guards the datasets of `/`, home and
  the working directory. It does not revert a change in another nested
  dataset.
- A view also captures changes of other sessions in the same directories
  while the program runs. `undo` would revert them too.
- When there are more conflicts than the file system lists (over 1024),
  the line ends with `SH_CONFLICT` and the apply cannot be undone (the
  shell does not know for sure what was applied).

**Safeguards:**

- `undo` never touches a nested dataset: a view sees only its own
  dataset, syncing skips nested datasets (even when syncing a whole
  directory).
- After a program the shell first commits the changes in the file system
  (`COMMIT`) and only then asks the journal - the journal shows only
  committed entries.
- Reads in a line with a clone do not ask the file system for the
  dataset: they take the clone above the path and repeat exactly only at
  the boundary of a nested dataset (`NOT_SUPPORTED`).
- Discarding a clone (freeing blocks) is deferred until the shell is idle;
  the result of the command is in the terminal right away.
- A program in `preview` also gets clones of the nested datasets of home
  and the working directory.

## Terminal and session service

<figure class="diagram">
<svg viewBox="0 0 760 300" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Session service">
<defs><marker id="sh3" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="box" x="10" y="30" width="170" height="60" rx="6"/><text class="t b" x="95" y="55" text-anchor="middle">terminal window</text><text class="m" x="95" y="74" text-anchor="middle">(6c GUI, test, later SSH)</text>
<path class="ln" d="M180 60 L248 60" marker-end="url(#sh3)"/>
<text class="m" x="184" y="50">protocol</text>
<rect class="acc" x="250" y="30" width="150" height="60" rx="6"/><text class="t b" x="325" y="55" text-anchor="middle">termd</text><text class="m" x="325" y="74" text-anchor="middle">only routes messages</text>
<path class="ln" d="M400 50 L468 40" marker-end="url(#sh3)"/>
<path class="ln" d="M400 75 L468 110" marker-end="url(#sh3)"/>
<rect class="box" x="470" y="15" width="150" height="50" rx="6"/><text class="t b" x="545" y="36" text-anchor="middle">shsess 1</text><text class="m" x="545" y="54" text-anchor="middle">interpreter, undo, jobs</text>
<rect class="box" x="470" y="85" width="150" height="50" rx="6"/><text class="t b" x="545" y="106" text-anchor="middle">shsess 2</text><text class="m" x="545" y="124" text-anchor="middle">own process</text>
<path class="ln" d="M620 40 L668 75" marker-end="url(#sh3)"/>
<path class="ln" d="M620 110 L668 85" marker-end="url(#sh3)"/>
<rect class="box" x="670" y="55" width="80" height="50" rx="6"/><text class="t b" x="710" y="78" text-anchor="middle">fsd</text><text class="m" x="710" y="95" text-anchor="middle">clones</text>
<rect class="warn" x="250" y="170" width="150" height="56" rx="6"/><text class="t b" x="325" y="193" text-anchor="middle">session registry</text><text class="m" x="325" y="211" text-anchor="middle">spare ends</text>
<path class="ln dash" d="M400 198 L545 137" marker-end="url(#sh3)"/>
<path class="ln dash" d="M325 90 L325 168" marker-end="url(#sh3)"/>
<text class="m" x="332" y="140">Park / Claim</text>
<text class="m" x="10" y="262">A crashing session does not bring down the others or termd. termd crashes: sessions and jobs keep running, a new termd</text>
<text class="m" x="10" y="280">takes them over from the registry (Adopt) and the window attaches to them (Attach) with block replay.</text>
</svg>
<figcaption>Each session in its own process; termd is only a broker, the registry keeps sessions across its restart.</figcaption>
</figure>

**A process per session** (`libs/term/src/broker.rs`, `libs/term/src/ctl.rs`):

- termd (`user/termd`) has no interpreter. It receives the window's
  messages and routes them by session number to the `shsess` process
  (`user/shsess`), one per session. Block and prompt numbers of session `N`
  lie in the range `N << 32`, so an answer to a prompt (`Answer`) can be
  routed without a table.
- **A crashing session** does not bring down the other sessions or termd;
  the window gets `Fail` "relace N skoncila".
- **Session registry:** besides the main channel to termd every session has
  a spare channel. termd stores its other end in the registry (`Park`). The
  registry is held by the process that starts termd (today a test in init,
  later the supervisor).
- **termd ends or crashes:** the sessions detach. A running command and
  background jobs finish and the output is stored in blocks. After its
  start a new termd collects the spare ends (`Claim`), sends `Adopt` on
  them and the sessions move to the new termd. The window then attaches
  (`Attach`) with block replay. Without a registry the sessions finish and
  end.
- **Clones aside** carry the session's tag in their name
  (`/tmp/.aside/<tag>-<n>`). A new termd discards clones of sessions that
  no longer run (a crash in the middle of a line). That is why only one
  termd (one registry) may run in the system.

⚠️ **Limit:** `Sessions` reports the working directory from the last
`Attach` (termd does not ask the session, so it does not block on a busy
session).


### Native protocol (`libs/term/src/proto.rs`)

Binary messages (1 byte kind, little-endian fields, text = length + UTF-8).
The window sends `Hello`, `Attach` (0 = new session), `Detach`, `Run`,
`Interrupt`, `Complete`, `Answer`, `Format`, `Sessions` and `Highlight`.
The service sends `BlockStart`, `Item`, `BlockEnd`, `Prompt`,
`Completions`, `Formatted`, `Highlighted`, `Cwd` (the current directory
after each block, so the prompt changes right after `cd`) and `Fail`.

The grey suggestion at the end of the line: for the command name from the
history, for an argument from completion by the argument type (`cd` only
directories, paths, table fields) - the window requests it itself
(`Complete` with the `seq` flag 0x8000_0000). Tab: a single match is
completed at once, several matches open a menu that typing narrows (arrows
/Tab choose, Enter confirms, Esc closes). The output scrolls with the wheel
and Shift+PgUp/PgDn (plain PgUp is the table); when not at the bottom a
"N lines below" bar shows it, a new block or typing returns to the bottom.
`help` is an overview by groups (name and description), details in
`help <command>`; a table row wraps only within the description cell.

- **Matching replies (version 2):** `Complete` and `Format` carry a request
  number `seq`, which `Completions` and `Formatted` return. Events of other
  blocks (`async` tasks, `bg`, the late end of a cancelled task) may arrive
  between a request and its reply. The window therefore matches replies by
  `seq` and drops a reply to an older request. `seq` of `Highlight` works
  the same way.
- **Block items:** a typed value, a note, a warning, an error (code,
  description, report with the location), a diff, progress, a link.
- **Values are sent typed.** A list of records with the same columns goes
  as a **table with a schema**: the column names once, then the rows. The
  terminal knows that `size` is a size and `mtime` a time and shows them its
  own way (alignment, units, local time). Conversion to text happens only at
  the very end.
- **VT100 is only an adapter** (`libs/term/src/vt.rs`): tables as aligned
  text, errors in red, notes dimmed, links as OSC 8. For the serial line and
  foreign programs.

### Sessions and blocks

- Every command is a **block** (line, items, result, duration). A block can
  be collapsed, copied, rerun or two runs compared; the terminal does that,
  the service holds the data.
- **Sessions belong to the service, not the window.** `Detach` (a window closed or crashed)
  does not end the session: a running command finishes, its output is
  stored in blocks and replayed on `Attach`. A question without an attached
  window is safely refused.
- **Ctrl-C:** the window sends `Interrupt`. The interpreter checks for
  interruption continuously (statements, loops, `where`/`each` items,
  `sleep` in 50 ms steps).
- **Completion by type** (`Complete`): command and function names in the
  command position, flags from the signature after `-`, variables after
  `$`, input fields in the `where` condition and for
  `sort`/`select`/`group`/`sum` (schema from the previous command's
  signature), file system paths elsewhere.
- **History with context:** line, working directory, result, duration and
  time (`history`, search via `History::search` by text and directory).
- **Error explanations:** `explain NOT_FOUND`, `explain SH_PARSE`,
  `explain -8` or the text of a log message. The texts come straight from
  [errors.md](errors.md) (Czech `chyby.md`), from this page and from
  [messages.md](messages.md).
- **Notation conversion** (`Format`): the terminal can rewrite a line into
  the other notation.

### Screen and font

`libs/term/src/screen.rs` keeps a text grid (it understands the output of
the VT100 adapter) and draws it into a linear framebuffer with the Noto Sans
Mono font, including accented letters. From 1080 lines up a larger font is
used. The same screen will serve as the rescue text console.

### Terminal colors

> **Change from the first shell version:** the terminal has colors. The
> protocol carries no colors and no ANSI, only **semantic classes**. A
> theme assigns the color at the end of the path (window, serial line,
> documentation). Highlighting takes its tokens from the lexer of
> `libs/shlang` (the single source of truth for the terminal and this web).

**Two levels of highlighting** (`shlang::highlight`):

- **Lexical, right in the window** on every key press, without a message.
  A word in command position is a command, then flags, strings, numbers,
  comments, keywords and pipes. A block line in the output and the
  examples in the documentation are highlighted the same way.
- **Semantic, from the service** (`Highlight` -> `Highlighted`, ordered by
  `seq`). The service knows the commands, functions, programs and
  variables of the session. An unknown command gets the class `unknown`
  (underlined while typing), a variable `var`, a field in a `where`
  condition `field`. It also returns the place of a parser error. An
  unfinished line (open string, pipe at the end) reports no error. The
  window drops answers with an older `seq`, so letters never wait for an
  answer.

| Class | What | Example |
|---|---|---|
| `cmd` | known command, function, program | `ls`, `where`, `.count()` |
| `unknown` | unknown command (underlined) | `lss` |
| `kw` | keyword | `let`, `for`, `try`, `true` |
| `flag` | flag | `-r`, `--depth` |
| `var` | variable | `$f`, `it` |
| `field` | record field | `it.size`, `size` in `where` |
| `str` | string | `"hello"` |
| `interp` | braces of an embedded expression | `"{f.name}"` |
| `num` | number with a unit | `5MB`, `2s`, `1.5` |
| `path` | path, pattern, redirect target | `~/photos`, `*.jpg` |
| `comment` | comment | `# note` |
| `op` | operator, bracket | `>`, `&&`, `=>` |
| `pipe` | pipe | `\|` |

```shell-b
ls ~/photos -r | where size > 5MB | sort mtime   # large photos
```

```shell-b
for f in (ls *.jpg) { echo "processing {f.name}" }
```

**Colored listings** (`term::render`): tables are colored by the types of
the values and the schema, not by ANSI in programs.

- **File name by kind:** `dir`, `dataset` (nested dataset: `kind`
  stays `"dir"`, field `dataset = true`), `symlink`, `exec` (the right to run `x` in the
  `access` field), `image`, `archive`, `document`, `media` (by extension),
  `hidden` (dot), `noaccess` (no right to read).
- **Size and age on a color scale** (4 steps): size up to 1 MB, 100 MB,
  1 GB and more; age up to a day, 30 days, a year and older.
- `true` green, `false` and errors in the error color, `null` muted,
  changes in a diff by kind.
- `ls` has new fields `dataset` (root of a nested dataset) and `access` (the session's rights `rwx` from fsd,
  `LIST_STAT_PAGE` sends them with every entry).

**Theme** (`term::theme`): built-in dark (default) and light. The user
overrides them with the file `~/.config/term/theme.toml`; an unknown class
or a bad color is an error with a line number.

```toml
base = "dark"            # or "light"
[syntax]
cmd = "#7ee2a8"
unknown = "#ff8a80 underline"
[files]
dir = "#7fb4ff bold"
[ui]
bg = "#15171c"
scale3 = "#ff8a80"       # the highest step of the scale
[ext]
image = "jpg png heic"   # extensions for a file kind
```

**End of the path:** the VT100 adapter draws 24-bit colors
(`38;2;r;g;b`), bold and underline; the screen (`term::screen`) can draw
them into the framebuffer. The class colors on this web (`sh-*`, `fk-*`)
are generated by `xtask` from the same theme. A test makes sure the
highlighting in the documentation and in the terminal gives the same
spans.

An example that cannot be converted to the other notation (mixed B and C,
a script with a header) has the language `mos` in the documentation: it is
highlighted the same way, only without conversion.

⚠️ **Plan:** coloring the user's own text (e.g. `echo` with a style class)
is a follow-up proposal.

### Interactive terminal and console

> **Change from the first shell version:** the terminal is not a "console
> from the 70s". Results are interactive tables, completion is a menu,
> progress changes in place in real time and prompts are elements of the
> protocol. The logic lives in the library `term::ui` (no system calls,
> host tests), so the text console and the GUI 6c window use the same.

<figure class="diagram">
<svg viewBox="0 0 760 230" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Console and interactive terminal">
<defs><marker id="sh4" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="box" x="10" y="20" width="140" height="56" rx="6"/><text class="t b" x="80" y="44" text-anchor="middle">inputd</text><text class="m" x="80" y="62" text-anchor="middle">focus = keys</text>
<rect class="box" x="10" y="140" width="140" height="56" rx="6"/><text class="t b" x="80" y="164" text-anchor="middle">kernel</text><text class="m" x="80" y="182" text-anchor="middle">FRAMEBUFFER, FB_CONTROL</text>
<path class="ln" d="M150 48 L248 90" marker-end="url(#sh4)"/>
<path class="ln" d="M150 168 L248 130" marker-end="url(#sh4)"/>
<rect class="acc" x="250" y="60" width="220" height="100" rx="6"/><text class="t b" x="360" y="84" text-anchor="middle">user/console</text><text class="m" x="360" y="104" text-anchor="middle">term::ui (editor, tables,</text><text class="m" x="360" y="120" text-anchor="middle">menu, prompts, progress)</text><text class="m" x="360" y="140" text-anchor="middle">term::screen -> framebuffer</text>
<path class="ln" d="M470 110 L568 110" marker-end="url(#sh4)"/>
<text class="m" x="478" y="102">native protocol</text>
<rect class="box" x="570" y="82" width="180" height="56" rx="6"/><text class="t b" x="660" y="106" text-anchor="middle">termd + shsess</text><text class="m" x="660" y="124" text-anchor="middle">sessions, interpreter</text>
<path class="ln dash" d="M360 60 L360 22" marker-end="url(#sh4)"/>
<text class="m" x="372" y="30">supervisor / test: display, focus, release</text>
</svg>
<figcaption>The console gets the display and focus from their owner; it never takes them itself.</figcaption>
</figure>

**Protocol** (an addition to the native protocol):

- `ItemUpdate { session, block, index, item }`: an item of a block changes
  in place. A progress bar thus changes in real time and the window
  computes the rate and the remaining time itself from the times of the
  updates.
- `Prompt { …, choices }` + `Choose { prompt, index }`: a prompt with
  choices (empty `choices` = yes/no and the answer `Answer`).
- The interpreter reports progress (`Host::progress`): `for` and `each`
  over a list of 20 items or more (updates only when the percentage
  changes) and `sleep` of 1 s or more.

**Window** (`term::ui::App`):

- **Line editor:** immediate highlighting, semantic from the service by
  `seq`; a gray suggestion from history (right arrow or End accepts it);
  history with the arrows; Ctrl-W word, Ctrl-U line, Ctrl-F convert B/C;
  Tab completion (a single option is completed right away, several = a
  menu).
- **Task panel:** Ctrl-T opens the task list (arrows, `c` cancel, `p`
  pause, `r` resume, Esc close); the status line shows the number of
  running tasks and a notice on completion or failure.
- **Blocks:** every command with a colored state mark (running, done,
  error), its run time and a spinner while running. Ctrl-L collapses all,
  space collapses the selected one.
- **Interactive table:** PgUp from the editor enters the last table. The
  arrows and PgUp/PgDn select a row, `s` sorts by the next column, `S`
  reverses the order, `/` filters (a substring in any cell). An action on
  a row runs as an ordinary line, so history, preview, the lock and undo
  apply: Enter opens a file (`open`) or a directory (`cd …; ls`), `c`
  inserts the path into the line and `d` deletes with a preview
  (`preview { rm … }`).
- **Prompts:** Yes/No buttons or choices, arrows and Enter, `a`/`n`,
  digits for choices.
- **A line entered before attaching** to a session waits and is sent
  after `Attached`.

```shell-b
for f in (ls ~/photos -r) { echo "checking {f.name}" }
```

**Console** (`user/console`): the text and rescue console for running
without GUI, a GUI crash and emergency mode (in the GUI the terminal is
the window `gui-term`). It never takes the display or focus itself. Their
owner (the supervisor, for now a test in init) hands them over the same
way as to the compositor. Control commands are text with the answer
`ok` / `err CODE` like with inputd:

| Command | Handles | What |
|---|---|---|
| `display` | framebuffer | take over the display (`FB_CONTROL` `CLAIM`) and draw |
| `focus` | channel of events from inputd | take keys |
| `release` | - | stop drawing, give the display back to the kernel; the answer carries the framebuffer, the focus channel closes |
| `status` | - | `ok drawing=… focus=… session=… connected=…` |
| `dump` | - | `ok` + text of the screen (tests, diagnostics) |

It draws into an image in memory and copies only the changed text lines
to the framebuffer (while typing usually just the editor line), only on a
change or while a command runs (at most about 7 frames per second). A file
name in an action on a row is always text: an embedded expression `{…}` is
escaped.

⚠️ **Limits:** the mouse (click on a row, a link) is not supported yet;
the display and focus in the normal run will be split by the supervisor
together with GUI 6c.

### Integration status

| What | Status |
|---|---|
| language, formatter, interpreter, undo, preview, lock, needs | done, host tests (`cargo test -p shlang`) |
| protocol, session service, broker, registry, VT100, screen, mounted paths | done, host tests (`cargo test -p term`) |
| `termd` and `shsess` in initfs | done; sessions in their own processes, file system over `fsproto` (clones aside, views, `LIST_STAT_PAGE`), output mirrored to the log |
| QEMU test | done: `user/init/src/tests/shell.rs` starts termd with its own fsd session and checks writes (draft in a clone), undo via an fsd view, Ctrl-C, notation conversion, block replay after detaching and that a session and its job survive a termd crash and a new termd takes them over from the registry; starting termd outside tests belongs in the init supervisor (later) |
| `async`/`await` | done: tasks in `shsess` threads with their own fsd session; tests `cargo test -p term` (std threads) and a QEMU test (concurrency, a task's undo, deadline, cancellation) |
| keyboard input | done: the console gets the focus from `inputd` (the test in init types with synthetic input) |
| drawing into the framebuffer | done: `user/console`, framebuffer from the kernel (`kind::FRAMEBUFFER`, `FB_CONTROL`); the QEMU test types `echo ahoj` and checks the screen; starting the console outside tests belongs in the supervisor |
| starting programs with their own fsd session | permission mapping done (`term::perm`, also from clones for `preview`); no program with a shell interface yet |

### Large files: the "file" value

`open` of a file over 64 KiB does not read it whole: it returns a value of
type `file` (path, size, kind by the first 512 B, e.g. "obrázek BMP
1920×1080"). The content is read only by a command that needs bytes or text
on its input (`from text`, `lines`, `save`, `count`...); `describe`, display
in the terminal and storing in a variable do not read it. The fields
`size`, `kind` and `path` can be read directly. **[implemented]** Streaming
(a pipeline that reads a large text in parts and stops early, e.g.
`lines | first 10`) **[designed]**.

### Lines in threads: Ctrl+Z and background

A session line runs in its own thread of the session process (`shsess`)
over the session interpreter and its own connection to fsd (same identity;
the draft and the undo views are FS objects, not connections). Meanwhile the
main thread serves the window: Ctrl+C, Ctrl+Z, prompts, further lines (a
queue) and environment services (sound, agent, task overview) that the line
thread does not call itself. Code: `libs/term/src/fg.rs` (line thread) and
`libs/term/src/session/line.rs` (session handling). **[implemented]**

```text
window ──Run/Interrupt/Background──▶ shsess main thread ──▶ line thread
  ▲                                    │  message box, prompts,   (session
  └──────── blocks, prompts ───────────┘  environment requests     interpreter)
```

- **Ctrl+Z** (message `Background`): the line keeps running as a background
  job and the session continues with a copy of the state from the start of
  the line (variables, functions, folder, history). What the job changes in
  variables and the folder does not show up in the session (like a
  subshell). The undo of earlier lines stays with the job; after Ctrl+Z the
  session starts with an empty undo.
- **`command &`**: straight to the background (the job gets a copy of the
  state, the session does not change).
- **`jobs`, `fg N`, `kill N`**: session job control; `fg` brings a job back
  to the foreground (Ctrl+C interrupts it, Ctrl+Z sends it back). `bg N`
  only reports that the job already runs (a stopped job does not exist yet).
- The end of a background job is announced by a line in its block:
  "[1] hotovo: ...", "[1] přerušeno: ..." or "[1] skončilo chybou: ...".
- A line typed meanwhile goes to a queue (visible in the window) and runs
  after the foreground line, in order.
- Without threads in the environment (tests) a line runs in the main thread
  as before.

### Window look by the design (design/terminal.html)

The `gui-term` window draws text with a character grid (the same frame as
the console) and, by the line roles (`term::ui::draw::Role`), draws the
design elements over it (`user/gui-term/src/rows.rs`). Screenshot for
comparison: `target/shots/terminal.png` (look test `look_shots`).

| Design element | Status |
|---|---|
| block left bar by state (running, done, error), bold command, time and result on the right in the UI font | **[implemented]** |
| error box (danger-soft, danger border) | **[implemented]** |
| input row: a line above it, a pill with a folder icon and name | **[implemented]** |
| change preview (dangerous operation lock, `preview`): a box with the list of changes (at most 6, then "... and N more") and buttons "Take N changes" / "Discard"; the buttons are in accessibility too, the console shows the list of changes as well | **[verified in QEMU]** |
| key hints as chips | **[implemented]** |
| a line under the table header | **[implemented]** (no column type under the name yet) |
| completion as a menu above the line, mouse, text selection and clipboard | **[designed]** (step 4, 6c components) |
| notation switch B / C in the title (state `App::notation_c`; a click, Ctrl+F and accessibility convert the line being typed) | **[verified in QEMU]** |
| a new block fades in (`motion.base`, curve `ease_enter`); blocks replayed after connecting do not. The grid does not do the 6 px shift from the design (the fade is also the substitute for "Reduce motion") | **[implemented]** |
| task panel on the right (width `size.side_panel`): task cards with state, progress bar and a Cancel button, a list in accessibility. It shows by itself only on a screen wider than 1280 px and in a window at least 900 px wide, otherwise Ctrl-T shows the tasks. Default window 1040 px (the compositor clips it to the screen; the limit to 85 % of the screen width waits for window resizing from the application) | **[implemented]** (QEMU tests have a 1280 px screen, the panel was verified once with a lowered limit) |
| session tabs in the title: a tab for each session of the service (the others labelled "detached"), a + button for a new session; a click and accessibility switch the session (detach and attach another). The replay after a switch does not freeze: the session first sends the last 12 blocks, then `Attached` and older blocks in batches of 16 from the newest, the window inserts them before newer ones by block number and handles at most 64 messages per frame | **[verified in QEMU]** |
| block actions (fold, run again; copy after the `clipproto` clipboard) | **[designed]** (step 5) |
| block action "compare with the previous run": output difference against the last run of the same command | **[designed]** (a separate piece after step 5) |

### Activity indicator and progress

Commands do not draw activity with characters themselves: they report it
in a structured way (a running block, a `Progress { done, total }` item)
and the window draws it with one shared module
`libs/term/src/activity.rs`. So all commands (`cp`, downloads, scrub...)
behave the same.

| Where it is drawn | Activity | Progress | Status |
|---|---|---|---|
| terminal window (`gui-term`) and the fallback console | a ring with a rotating arc in the accent color (1 cell, ~16 frames/s) | a smooth bar with partial cells in 1/8 steps, percent, rate, time estimate | **[implemented]** |
| foreign terminal, text fallback | braille dots `⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏`, at most 10 frames/s | a bar of `▏▎▍▌▋▊▉█` | **[implemented]** |
| "Reduce motion" | a static `◌` (running) / `●` (done) | whole cells only | **[designed]** (the mode exists, the system switch does not yet) |
| output that is not a terminal (file, script) | no animation | only the final state | **[implemented]** |
| progress from foreign programs via OSC 9;4 | | as above | **[designed]** |

The window and the console draw the ring and the partial cells
themselves, because the fonts lack these characters. On the wire they are
characters from the Unicode private use area (U+F0000 ring, U+F0100
partial bar cell) and `activity::coverage` computes the same pixel coverage
for both. The ring is redrawn only while something runs.

## Concurrency: async and await

> **Done (variant A):** a task `async { }`, waiting `await` (in C also
> `x.await`), `await all [...]`, `await any [...]`, a `timeout`,
> cancellation `cancel` and parallel `each --parallel N`. The keywords are
> the same in both notations.

```shell-b
let a = async { ls ~/Photos -r | where ext == "jpg" | count }
let b = async { ls ~/Documents -r | count }
let (photos, documents) = await all [a, b]
let first = await any [a, b] timeout 5s
```

```shell-b
let c = async { open ~/data.json | from json }
let r = try await c
if r.failed { echo "cannot: {r.error.message}" }
```

```shell-b
ls ~/Photos | each --parallel 4 f => { open $f.path | from text | lines | count }
```

The value of `async { }` is a task handle (the record `{task: n}`); `await`
returns the block's result. `await all` returns a list in the order of the
list (`let (a, b) = ...` destructures it), `await any` the result of the
first task done.

**Rules:**

- **Structured concurrency.** A task does not outlive the block it was
  created in (a script, a function call): when the block ends (also with an
  error or Ctrl-C), running tasks are cancelled and the block says so in a
  note. A task from the command line belongs to the session, so a later line
  can still wait for it. The exception is an explicit `bg`.
- **Errors:** a task's error shows up at `await` and propagates by itself;
  `let r = try await a` returns it as a value. `await all` stops at the
  first error and cancels the other tasks.
- **Deadline:** `await ... timeout 5s` returns the error `TIMED_OUT` after
  the deadline; the task keeps running (`cancel` cancels it).
- **Ctrl-C** during `await` cancels the tasks being waited for.
- **Files:** a task has its own draft of changes. The changes are committed
  when the task finishes and get their own `undo` entry in the session. A
  cancelled or failed task discards its draft, nothing changes. The lock on
  dangerous operations applies to a task too (a prompt in the terminal,
  refused without a window).
- **Permissions do not widen:** a task inherits the caller's `needs`
  manifest, working directory and lock.
- **Copies of variables:** a task sees a copy of the variables from the time
  it was created (a later change does not affect it). A closure cannot be
  passed into a task (`SH_TYPE`); define it inside `async { }`.
- **Terminal:** every task has its own block (`async ...`) with output,
  progress and result. The blocks are replayed after the window detaches and
  attaches again.

**Underneath** (the session process `shsess`):

- every task runs in **its own thread** of the session process
  (`libos::thread`, 256 KB stack) with its own interpreter and its own fsd
  session (`CONNECT`: the same identity and paths), so the task's draft does
  not mix with the line,
- values cross threads encoded the same way as in the terminal protocol
  (the interpreter uses `Rc`, which threads must not share),
- the task thread reports output to a shared table and wakes the main
  thread with a **kernel event**; the main thread waits for a window message
  or the event (`wait_many`), `await` waits for the event with a deadline.
  No busy waiting,
- `cancel` sets a flag the task's interpreter reads (like Ctrl-C).

```text
window ──Run──▶ shsess (main thread: the line's interpreter)
                   │ async { } ─▶ task thread (interpreter, fsd session, draft)
                   │                 │ output, prompts, result ─▶ task table
                   │ ◀── event ──────┘
                   └─▶ task blocks to the window, await takes the result and undo
```

⚠️ **Limits:**

- a host without threads (interpreter tests) runs tasks **in place** one
  after another; the results are the same, just without concurrency,
- a task does not start programs (`exec` is only in the main thread so far),
- switching between running tasks in the window is not there yet (blocks
  are stacked),
- two tasks changing the same file end with the conflict `SH_CONFLICT` for
  the one that finishes second (like two lines in different windows).

| Variant | Notation | Why not |
|---|---|---|
| **A (chosen)** | `async`/`await`, `await all/any`, `each --parallel` | - |
| B | `&` and `wait` like in bash | no return values, errors get lost |
| C | only `each --parallel`, `par { }` | cannot wait for any or the first task done |

## Shell errors

Shell error codes (`SH_*`). System errors (`NOT_FOUND`, ...) are in
[Error codes](errors.md). `explain CODE` shows the text from the Czech
version of this page. A test in `libs/shlang` checks completeness (every
code in cs and en).

### `SH_PARSE` (syntax)

**Meaning:** the source cannot be parsed.

**When:** a missing bracket or quote, an unknown character, `$` in an
expression, a redirect in the wrong place, a variable named like a field in
the `where` condition.

**What to do:** the report shows the line, the column and a hint.
`fmt --style c` shows how the parser understood the command.

### `SH_UNKNOWN` (unknown name)

**Meaning:** a variable, field, function or command does not exist.

**When:** a typo, a variable without `let`, a field the record does not
have (the report lists the fields it has).

**What to do:** the hint offers the closest name ("did you mean `ls`?").
List of commands: `help`.

### `SH_TYPE` (type)

**Meaning:** a value has a different type than the operation expects.

**When:** `size > 100` (missing unit), `"a" + 1`, bytes into `where`
(missing `from json`), records into a file (missing `to text`), a condition
that is not a `bool`.

**What to do:** convert explicitly (`from json`, `to text`, `lines`) or add
a unit (`100MB`, `30s`).

### `SH_ARGS` (arguments)

**Meaning:** a command got wrong arguments.

**When:** a required argument is missing, there are too many, an unknown
flag, a flag without a value, an index out of the list.

**What to do:** the hint shows the command usage; details: `help command`.

### `SH_NO_MATCH` (pattern matched nothing)

**Meaning:** a file pattern (`*.txt`) matched no file.

**When:** there is no such file in the directory. The shell never silently
passes the pattern as text.

**What to do:** check the directory and the pattern; if it is intended,
handle it with `try` or `||`.

### `SH_INTERRUPTED` (interrupted)

**Meaning:** the user pressed Ctrl-C.

**When:** any time during a command. It cannot be caught by `try` or `||`.

**What to do:** nothing; changes of the interrupted line are applied only
up to the interruption (and can be reverted with `undo`).

### `SH_NEEDS` (manifest)

**Meaning:** a script or program wants something its `needs` manifest does
not allow.

**When:** reading or writing outside the listed paths, a script wants more
than its caller, a program started inside `preview`.

**What to do:** add the path to `needs { read: [...] write: [...] }`. Run
the program outside `preview`.

### `SH_REJECTED` (rejected)

**Meaning:** the user rejected the changes or permissions.

**When:** answering "no" in a preview, at the dangerous operation lock or
to a script's permissions; a question without an attached window.

**What to do:** nothing changed. Run it again and confirm, or use `preview`
to inspect the changes.

### `SH_NO_UNDO` (nothing to undo)

**Meaning:** `undo` has no applied command.

**When:** nothing in the session has changed files yet, or everything has
already been reverted (16 commands are remembered).

**What to do:** nothing.

### `SH_ARITH` (arithmetic)

**Meaning:** division by zero or number overflow.

**When:** `x / 0`, a result outside a 64-bit integer.

**What to do:** check the divisor; use floats for large numbers.

### `SH_USER` (script error)

**Meaning:** the script raised the error with the `error` command.

**When:** `error "message"` without `--code`.

**What to do:** follow the script's message.

### `SH_FORMAT` (data format)

**Meaning:** data cannot be converted.

**When:** invalid JSON in `from json`, bytes that are not UTF-8.

**What to do:** the report gives the position in the data.

### `SH_LANG` (language version)

**Meaning:** the script wants a language version this shell does not know.

**When:** `lang 2` and higher.

**What to do:** update the system or rewrite the script for `lang 1`.

### `SH_DEPTH` (depth)

**Meaning:** too deep nesting or recursion.

**When:** endless recursion of a function, an extremely nested expression.
It is guarded by stack use, not by the number of calls.

**What to do:** rewrite the recursion as a loop (`for`, `while`).

### `SH_CONFLICT` (conflict)

**Meaning:** part of a line's changes was not applied because the file
had meanwhile changed in another way too (another session, a program).

**When:** when a line's draft or a preview is applied. Applying is
three-way: against the state the clone started from. What changed in the
original only elsewhere is applied; what changed on both sides stays as it
is in the original.

**What to do:** the other changes are written and `undo` reverts them.
Repeat the command over the current state of the files in the message.
