# Shell language and smart terminal

> Status: **agreed and implemented (milestone 6b)** - [two notations](#decision-one-language-two-notations-b-and-c),
> [notation decisions](#notation-decisions) and [grammar decisions](#grammar-decisions).
> The language is described in [Grammar](#language-grammar-lang-1) and
> [Semantics](#semantics), the terminal in [Terminal and session service](#terminal-and-session-service).
> The research and variants chapters show what was chosen from.
>
> Code: `libs/shlang` (language, `no_std` + `alloc`, host tests),
> `libs/term` (protocol, sessions, VT100, screen), `user/termd` (service).

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
  budget; termd has a 256 KB stack and the interpreter may use 160 KB.
  Exceeding it gives `SH_DEPTH`, not a crash.

### Language version

`lang 1` applies to both notations. A script without `lang` runs in the
newest version with a warning. An unknown version is the error `SH_LANG`.
An incompatible change = a new version; an old script keeps running in its
own.

## Built-in commands

Every command has a short name and a full `verb-noun` name (an alias).
`help` lists them, `help ls` shows usage and flags.

| Short | Full | What it does |
|---|---|---|
| `ls` | `file-list` | files as the table `{name, path, kind, size, mtime, ext}`; `-r`, `-a` |
| `cd`, `pwd` | `dir-change`, `dir-current` | working directory |
| `open` (`cat`) | `file-read` | a file as bytes |
| `save` | `file-write` | saves bytes or text; `-a` appends |
| `rm`, `cp`, `mv`, `mkdir` | `file-remove`, `file-copy`, `file-move`, `dir-create` | `-r`, `-p` |
| `exists`, `df` | `file-exists`, `fs-info` | |
| `where`, `each`, `select`, `sort`, `first`, `last`, `count`, `sum`, `group`, `reverse`, `range` | `list-filter`, `list-map`, ... | working with tables and lists |
| `echo`, `log`, `lines`, `split`, `join`, `trim`, `upper`, `lower`, `contains`, `starts-with`, `ends-with`, `replace` | `text-...` | text |
| `from`, `to`, `describe` | `data-from`, `data-to`, `value-type` | explicit conversions (`json`, `text`, `lines`) |
| `now`, `sleep` | `time-now`, `time-sleep` | |
| `undo`, `preview` | `command-undo`, `command-preview` | see [Files](#files-lazy-undo-preview-and-lock) |
| `jobs`, `wait`, `kill` | `job-list`, `job-wait`, `job-kill` | background jobs |
| `history`, `explain`, `help`, `run`, `error`, `exit` | `history-list`, `error-explain`, `help-show`, `script-run`, `error-raise`, `session-exit` | |

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

## Files: lazy undo, preview and lock

<figure class="diagram">
<svg viewBox="0 0 760 250" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="File changes of a command">
<defs><marker id="se2" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="box" x="10" y="80" width="130" height="56" rx="6"/><text class="t b" x="75" y="104" text-anchor="middle">command</text><text class="m" x="75" y="122" text-anchor="middle">rm, save, mv ...</text>
<path class="ln" d="M140 108 L188 108" marker-end="url(#se2)"/>
<rect class="acc" x="190" y="80" width="140" height="56" rx="6"/><text class="t b" x="260" y="104" text-anchor="middle">overlay</text><text class="m" x="260" y="122" text-anchor="middle">dry-run changes</text>
<path class="ln" d="M330 108 L378 108" marker-end="url(#se2)"/>
<rect class="box" x="380" y="80" width="120" height="56" rx="6"/><text class="t b" x="440" y="104" text-anchor="middle">diff</text><text class="m" x="440" y="122" text-anchor="middle">end of line</text>
<path class="ln" d="M500 95 L558 48" marker-end="url(#se2)"/>
<path class="ln" d="M500 122 L558 172" marker-end="url(#se2)"/>
<rect class="ok" x="560" y="20" width="190" height="56" rx="6"/><text class="t b" x="655" y="42" text-anchor="middle">≤ N files: apply</text><text class="m" x="655" y="60" text-anchor="middle">lazy snapshot first</text>
<rect class="warn" x="560" y="145" width="190" height="56" rx="6"/><text class="t b" x="655" y="167" text-anchor="middle">&gt; N or preview</text><text class="m" x="655" y="185" text-anchor="middle">show diff, confirm</text>
<path class="ln dash" d="M655 145 L655 78" marker-end="url(#se2)"/>
<text class="m" x="10" y="232">A read-only command costs nothing (the overlay is empty, no snapshot). undo restores the last applied changes from the snapshot.</text>
</svg>
<figcaption>Changes go to an overlay; they are applied at the end of the line with a lazy snapshot.</figcaption>
</figure>

- **Lazy undo:** every command writes to an overlay. At the end of the line
  the shell computes the diff. An empty diff = nothing happens (read-only
  commands cost nothing). Otherwise the shell **only now** creates a
  snapshot (`shell-undo-N`, a cheap CoW snapshot on FS v3) and applies the
  changes. `undo` reverts the last command and works repeatedly (the last 16
  commands). On a file system without snapshots the shell keeps the
  original content of changed files in memory.
- **"Show what will happen":** `preview { ... }` runs the block as a dry
  run, shows a table of changes (`added`, `modified`, `removed`,
  `dir-added`, `dir-removed`, `replaced` = a file replaced by a directory or
  the other way round) and applies them only after confirmation (`-y`
  without asking). Rejecting = `SH_REJECTED` and nothing changes.
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
- **Programs** get permissions from their manifest through their own fsd
  session (`CONNECT_AS` with mounted paths, `term::perm::mounts`; write
  overrides read, paths are mounted at the same place). Before a program
  starts, the shell applies the line's changes so far (the program sees the
  real file system), **with the same dangerous operation lock** as at the
  end of a line: rejecting = the program does not start. Then it takes a
  snapshot and remembers the directories the program may write to (the
  whole file system with full rights). When the program changed nothing,
  the undo entry is dropped, so `undo` reverts the previous command.
  Otherwise `undo` syncs those directories with the snapshot.

⚠️ **Limits:** the overlay keeps written data in memory until the end of
the line (large writes cost memory). Programs outside the shell do not see
the overlay, so `preview` refuses them (`SH_NEEDS`) until fsd can do
dataset clones and diffs over the protocol (`libs/fs` can, `fsproto` cannot
yet). A snapshot over `fsproto` is a snapshot of the whole volume, not only
of the affected dataset.

## Terminal and session service

<figure class="diagram">
<svg viewBox="0 0 760 260" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Session service">
<defs><marker id="se3" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="box" x="10" y="30" width="170" height="60" rx="6"/><text class="t b" x="95" y="55" text-anchor="middle">terminal window</text><text class="m" x="95" y="74" text-anchor="middle">(6c GUI, test, later SSH)</text>
<path class="ln" d="M180 60 L268 90" marker-end="url(#se3)"/>
<text class="m" x="185" y="56">native protocol</text>
<rect class="acc" x="270" y="50" width="220" height="110" rx="6"/><text class="t b" x="380" y="75" text-anchor="middle">termd</text><text class="m" x="380" y="95" text-anchor="middle">session: interpreter, cwd,</text><text class="m" x="380" y="111" text-anchor="middle">history, undo, jobs</text><text class="m" x="380" y="127" text-anchor="middle">output blocks</text><text class="m" x="380" y="143" text-anchor="middle">survive a detached window</text>
<path class="ln" d="M490 80 L578 60" marker-end="url(#se3)"/>
<rect class="box" x="580" y="30" width="170" height="56" rx="6"/><text class="t b" x="665" y="54" text-anchor="middle">fsd</text><text class="m" x="665" y="72" text-anchor="middle">files, snapshots</text>
<path class="ln" d="M490 125 L578 140" marker-end="url(#se3)"/>
<rect class="box" x="580" y="115" width="170" height="56" rx="6"/><text class="t b" x="665" y="139" text-anchor="middle">VT100 adapter</text><text class="m" x="665" y="157" text-anchor="middle">log / serial line</text>
<path class="ln dash" d="M380 160 L380 198" marker-end="url(#se3)"/>
<rect class="box" x="270" y="200" width="220" height="44" rx="6"/><text class="t b" x="380" y="220" text-anchor="middle">screen + own font</text><text class="m" x="380" y="236" text-anchor="middle">framebuffer (capability)</text>
</svg>
<figcaption>Sessions belong to the termd service; a window only attaches.</figcaption>
</figure>

### Native protocol (`libs/term/src/proto.rs`)

Binary messages (1 byte kind, little-endian fields, text = length + UTF-8).
The window sends `Hello`, `Attach` (0 = new session), `Detach`, `Run`,
`Interrupt`, `Complete`, `Answer`, `Format` and `Sessions`. The service
sends `BlockStart`, `Item`, `BlockEnd`, `Prompt`, `Completions`, `Formatted`
and `Fail`.

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
- **Sessions belong to the service.** `Detach` (a window closed or crashed)
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

### Integration status

| What | Status |
|---|---|
| language, formatter, interpreter, undo, preview, lock, needs | done, host tests (`cargo test -p shlang`) |
| protocol, session service, VT100, screen, mounted paths | done, host tests (`cargo test -p term`) |
| `termd` in initfs | done; file system over `fsproto`, output mirrored to the log |
| QEMU test | done: `user/init/src/tests/shell.rs` starts termd with its own fsd session and checks writes, undo via an fsd snapshot, Ctrl-C, notation conversion and block replay after detaching; starting termd outside tests belongs in the init supervisor (later) |
| keyboard input | waits for `inputd` (milestone 6a, branch not in `main`); until then the window or test sends lines |
| drawing into the framebuffer | library done; waits for the framebuffer to be passed as a capability (kernel, `userboot`) |
| starting programs with their own fsd session | permission mapping done (`term::perm`); no program with a shell interface yet |

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
