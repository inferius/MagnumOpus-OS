//! Podpisy prikazu: jmena (kratke i plne), argumenty, prepinace, vstup
//! a vystup.
//!
//! Parser zapisu B podle podpisu prevadi kratke prepinace na plna jmena
//! (`-r` -> `recursive: true`), pozna podminku `where` a hodnotove
//! prepinace (`--width 1600`). Doplnovani a napoveda pouzivaji totez.
//! Vestavene prikazy jsou v [`BUILTINS`]; programy dodaji podpis
//! z manifestu (viz [`Sigs`]).

use alloc::vec::Vec;

/// Druh argumentu (pro parser, kontrolu a doplnovani).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Any,
    Path,
    Str,
    Int,
    Duration,
    /// Closure (`f => ...`).
    Closure,
    /// Podminka nad radkem: v B cukr `where size > 1MB` (closure `it => ...`).
    Cond,
    /// Blok nebo closure bez parametru (`preview { ... }`).
    Block,
}

#[derive(Debug, Clone, Copy)]
pub struct Param {
    pub name: &'static str,
    pub kind: Kind,
    pub optional: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct Flag {
    pub long: &'static str,
    pub short: Option<char>,
    /// `None` = prepinac bez hodnoty (bool).
    pub value: Option<Kind>,
    pub help: &'static str,
}

/// Co prikaz cte a vraci.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Io {
    Nothing,
    Any,
    /// Bajtovy proud.
    Bytes,
    /// Text.
    Text,
    /// Seznam zaznamu s danymi poli (prazdne = nezname).
    Records(&'static [&'static str]),
}

#[derive(Debug, Clone, Copy)]
pub struct Sig {
    /// Kratke jmeno (`ls`).
    pub name: &'static str,
    /// Plne jmeno `sloveso-podstatne` (`file-list`), alias.
    pub full: &'static str,
    pub params: &'static [Param],
    /// Libovolne dalsi pozicni argumenty tohoto druhu.
    pub rest: Option<Kind>,
    pub flags: &'static [Flag],
    pub input: Io,
    pub output: Io,
    /// Ceska napoveda (jedna veta).
    pub help: &'static str,
}

impl Sig {
    pub fn flag_long(&self, long: &str) -> Option<&Flag> {
        self.flags.iter().find(|f| f.long == long)
    }

    pub fn flag_short(&self, c: char) -> Option<&Flag> {
        self.flags.iter().find(|f| f.short == Some(c))
    }

    /// Druh `i`-teho pozicniho argumentu.
    pub fn pos_kind(&self, i: usize) -> Option<Kind> {
        self.params.get(i).map(|p| p.kind).or(self.rest)
    }
}

/// Zdroj podpisu (vestavene + programy z manifestu).
pub trait Sigs {
    fn get(&self, name: &str) -> Option<&Sig>;
    /// Vsechna jmena prikazu (pro doplnovani).
    fn names(&self) -> Vec<&'static str> {
        Vec::new()
    }
}

/// Jen vestavene prikazy.
pub struct Builtins;

impl Sigs for Builtins {
    fn get(&self, name: &str) -> Option<&Sig> {
        builtin(name)
    }

    fn names(&self) -> Vec<&'static str> {
        BUILTINS.iter().flat_map(|s| [s.name, s.full]).collect()
    }
}

/// Vestaveny podpis podle kratkeho nebo plneho jmena.
pub fn builtin(name: &str) -> Option<&'static Sig> {
    BUILTINS.iter().find(|s| s.name == name || s.full == name || alias(name) == Some(s.name))
}

/// Dalsi aliasy (zvyk z jinych shellu).
fn alias(name: &str) -> Option<&'static str> {
    match name {
        "cat" => Some("open"),
        _ => None,
    }
}

/// Pole zaznamu souboru (vystup `ls`).
pub const FILE_FIELDS: &[&str] = &["name", "path", "kind", "size", "mtime", "ext", "access"];

const fn p(name: &'static str, kind: Kind) -> Param {
    Param { name, kind, optional: false }
}

const fn opt(name: &'static str, kind: Kind) -> Param {
    Param { name, kind, optional: true }
}

const fn flag(long: &'static str, short: char, help: &'static str) -> Flag {
    Flag { long, short: Some(short), value: None, help }
}

const NONE: &[Param] = &[];
const NOFLAGS: &[Flag] = &[];

macro_rules! sig {
    ($name:expr, $full:expr, $params:expr, $rest:expr, $flags:expr, $input:expr, $output:expr, $help:expr) => {
        Sig {
            name: $name,
            full: $full,
            params: $params,
            rest: $rest,
            flags: $flags,
            input: $input,
            output: $output,
            help: $help,
        }
    };
}

use Io::{Any as IA, Bytes as IB, Nothing as IN, Text as IT};

/// Vestavene prikazy jazyka verze 1.
pub const BUILTINS: &[Sig] = &[
    // soubory
    sig!("ls", "file-list", &[opt("path", Kind::Path)], None,
        &[flag("recursive", 'r', "i podslozky"), flag("all", 'a', "i skryte soubory (zacinajici teckou)")],
        IN, Io::Records(FILE_FIELDS), "vypise soubory ve slozce jako tabulku"),
    sig!("cd", "dir-change", &[opt("path", Kind::Path)], None, NOFLAGS, IN, IN,
        "zmeni pracovni slozku (bez argumentu na domovskou)"),
    sig!("pwd", "dir-current", NONE, None, NOFLAGS, IN, IT, "vrati pracovni slozku"),
    sig!("open", "file-read", &[p("path", Kind::Path)], None, NOFLAGS, IN, IB, "precte soubor jako bajty"),
    sig!("save", "file-write", &[p("path", Kind::Path)], None,
        &[flag("append", 'a', "pripojit na konec")], IA, IN,
        "ulozi bajty nebo text do souboru (zaznamy jen po `to text`/`to json`)"),
    sig!("rm", "file-remove", NONE, Some(Kind::Path), &[flag("recursive", 'r', "i slozky s obsahem")], IN, IN,
        "smaze soubory"),
    sig!("cp", "file-copy", &[p("from", Kind::Path), p("to", Kind::Path)], None,
        &[flag("recursive", 'r', "i slozky")], IN, IN, "zkopiruje soubor"),
    sig!("mv", "file-move", &[p("from", Kind::Path), p("to", Kind::Path)], None, NOFLAGS, IN, IN,
        "presune nebo prejmenuje soubor"),
    sig!("mkdir", "dir-create", NONE, Some(Kind::Path), &[flag("parents", 'p', "vytvorit i nadrazene slozky")],
        IN, IN, "vytvori slozku"),
    sig!("exists", "file-exists", &[p("path", Kind::Path)], None, NOFLAGS, IN, IA, "existuje soubor?"),
    sig!("df", "fs-info", &[opt("path", Kind::Path)], None, NOFLAGS, IN, Io::Records(&["total", "used", "free"]),
        "misto na souborovem systemu"),
    // text a vystup
    sig!("echo", "text-print", NONE, Some(Kind::Any), NOFLAGS, IN, IT, "vrati argumenty spojene mezerou"),
    sig!("log", "text-log", NONE, Some(Kind::Any), NOFLAGS, IN, IN, "vypise poznamku do terminalu"),
    sig!("lines", "text-lines", NONE, None, NOFLAGS, IA, Io::Records(&[]), "rozdeli text na radky"),
    sig!("split", "text-split", &[p("sep", Kind::Str)], None, NOFLAGS, IT, Io::Records(&[]), "rozdeli text"),
    sig!("join", "text-join", &[opt("sep", Kind::Str)], None, NOFLAGS, IA, IT, "spoji seznam textu"),
    sig!("trim", "text-trim", NONE, None, NOFLAGS, IT, IT, "odstrani mezery na okrajich"),
    sig!("upper", "text-upper", NONE, None, NOFLAGS, IT, IT, "velka pismena"),
    sig!("lower", "text-lower", NONE, None, NOFLAGS, IT, IT, "mala pismena"),
    sig!("contains", "text-contains", &[p("part", Kind::Any)], None, NOFLAGS, IA, IA,
        "obsahuje text (nebo seznam) danou cast?"),
    sig!("starts-with", "text-starts-with", &[p("prefix", Kind::Str)], None, NOFLAGS, IT, IA, "zacina textem?"),
    sig!("ends-with", "text-ends-with", &[p("suffix", Kind::Str)], None, NOFLAGS, IT, IA, "konci textem?"),
    sig!("replace", "text-replace", &[p("from", Kind::Str), p("to", Kind::Str)], None, NOFLAGS, IT, IT,
        "nahradi vsechny vyskyty"),
    // seznamy a tabulky
    sig!("where", "list-filter", &[p("cond", Kind::Cond)], None, NOFLAGS, IA, IA, "necha radky, ktere splni podminku"),
    sig!("each", "list-map", &[p("f", Kind::Closure)], None, NOFLAGS, IA, IA, "pouzije funkci na kazdy radek"),
    sig!("select", "record-select", NONE, Some(Kind::Str), NOFLAGS, IA, IA, "necha jen vybrane sloupce"),
    sig!("sort", "list-sort", &[opt("by", Kind::Str)], None, &[flag("desc", 'd', "sestupne")], IA, IA,
        "seradi radky (podle sloupce)"),
    sig!("first", "list-first", &[opt("n", Kind::Int)], None, NOFLAGS, IA, IA, "prvni radek (nebo prvnich n)"),
    sig!("last", "list-last", &[opt("n", Kind::Int)], None, NOFLAGS, IA, IA, "posledni radek (nebo poslednich n)"),
    sig!("count", "list-count", NONE, None, NOFLAGS, IA, IA, "pocet radku (nebo znaku textu)"),
    sig!("sum", "list-sum", &[opt("field", Kind::Str)], None, NOFLAGS, IA, IA, "soucet (sloupce)"),
    sig!("group", "list-group", &[p("by", Kind::Str)], None, NOFLAGS, IA, Io::Records(&["key", "items"]),
        "seskupi radky podle sloupce"),
    sig!("reverse", "list-reverse", NONE, None, NOFLAGS, IA, IA, "obrati poradi"),
    sig!("range", "list-range", &[p("from", Kind::Int), p("to", Kind::Int)], None, NOFLAGS, IN, IA,
        "cisla od..do (bez horni meze)"),
    // prevody (jen vyslovne)
    sig!("from", "data-from", &[p("format", Kind::Str)], None, NOFLAGS, IA, IA,
        "prevede bajty/text na hodnoty (json, text, lines)"),
    sig!("to", "data-to", &[p("format", Kind::Str)], None, NOFLAGS, IA, IA, "prevede hodnoty na text (json, text)"),
    sig!("describe", "value-type", NONE, None, NOFLAGS, IA, IT, "typ hodnoty"),
    // cas
    sig!("now", "time-now", NONE, None, NOFLAGS, IN, IA, "aktualni cas"),
    sig!("sleep", "time-sleep", &[p("time", Kind::Duration)], None, NOFLAGS, IN, IN, "pocka"),
    // relace, ulohy, bezpecnost
    sig!("undo", "command-undo", NONE, None, NOFLAGS, IN, IA, "vrati zmeny posledniho prikazu"),
    sig!("preview", "command-preview", &[p("body", Kind::Block)], None,
        &[flag("yes", 'y', "prevzit bez dotazu")], IN, IA,
        "spusti blok nanecisto, ukaze zmeny a po potvrzeni je prevezme"),
    sig!("jobs", "job-list", NONE, None, NOFLAGS, IN, Io::Records(&["id", "state", "command"]),
        "ulohy na pozadi"),
    sig!("wait", "job-wait", &[opt("id", Kind::Int)], None, NOFLAGS, IN, IA, "pocka na ulohu a vrati jeji vysledek"),
    sig!("kill", "job-kill", &[p("id", Kind::Int)], None, NOFLAGS, IN, IN, "ukonci ulohu"),
    sig!("history", "history-list", NONE, None, NOFLAGS, IN,
        Io::Records(&["line", "cwd", "ok", "duration", "time"]), "historie prikazu s kontextem"),
    sig!("explain", "error-explain", &[p("code", Kind::Str)], None, NOFLAGS, IA, IT,
        "vysvetli kod chyby nebo hlasku z logu"),
    sig!("help", "help-show", &[opt("command", Kind::Str)], None, NOFLAGS, IN, IT, "napoveda k prikazu"),
    sig!("run", "script-run", &[p("path", Kind::Path)], None, NOFLAGS, IN, IA,
        "spusti skript (kontroluje jeho manifest needs)"),
    sig!("error", "error-raise", &[p("message", Kind::Str)], None,
        &[Flag { long: "code", short: None, value: Some(Kind::Str), help: "kod chyby" }], IN, IN, "vyvola chybu"),
    sig!("exit", "session-exit", NONE, None, NOFLAGS, IN, IN, "ukonci relaci"),
];
