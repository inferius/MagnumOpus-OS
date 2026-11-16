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
pub trait Sigs: Send + Sync {
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
pub const FILE_FIELDS: &[&str] = &["name", "path", "kind", "size", "mtime", "ext", "access", "dataset", "items", "files", "dirs"];

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
/// Skupiny prikazu pro prehled `help` (poradi i nazvy pro cloveka).
pub const GROUPS: &[(&str, &[&str])] = &[
    ("Soubory", &["ls", "cd", "pwd", "open", "start", "save", "rm", "cp", "mv", "mkdir", "exists", "df"]),
    ("Text a výstup", &["echo", "log", "lines", "split", "join", "trim", "upper", "lower", "contains", "starts-with", "ends-with", "replace"]),
    ("Seznamy a tabulky", &["where", "each", "select", "sort", "first", "last", "count", "sum", "group", "reverse", "range"]),
    ("Převody a čas", &["from", "to", "convert", "describe", "now", "sleep"]),
    ("Úlohy", &["tasks", "jobs", "fg", "wait", "kill", "cancel"]),
    ("Relace a bezpečnost", &["undo", "preview", "history", "run", "error", "exit"]),
    ("Nápověda", &["help", "explain"]),
    (
        "Systém",
        &[
            "audio",
            "theme",
            "permissions",
            "acl",
            "sshd",
            "ssh-key",
            "remote-access",
            "system",
            #[cfg(feature = "dev-mode")]
            "devmode",
            "loglevel",
            "report",
            "install",
            "shutdown",
            "reboot",
        ],
    ),
];

pub const BUILTINS: &[Sig] = &[
    // soubory
    sig!("ls", "file-list", &[opt("path", Kind::Path)], None,
        &[flag("recursive", 'r', "i podsložky"), flag("all", 'a', "i skryté soubory (začínající tečkou)")],
        IN, Io::Records(FILE_FIELDS), "vypíše soubory ve složce jako tabulku"),
    sig!("cd", "dir-change", &[opt("path", Kind::Path)], None,
        &[Flag { long: "history", short: None, value: None, help: "poslední složky relace (cd -N se do nich vrátí)" }], IN, IA,
        "změní pracovní složku (bez argumentu na domovskou)"),
    sig!("pwd", "dir-current", NONE, None, NOFLAGS, IN, IT, "vrátí pracovní složku"),
    sig!("open", "file-read", &[p("path", Kind::Path)], None, NOFLAGS, IN, IB, "přečte soubor jako bajty"),
    sig!("start", "file-start", &[p("path", Kind::Path)], None, NOFLAGS, IN, IN,
        "otevře soubor nebo složku v aplikaci podle typu (jako dvojklik)"),
    sig!("save", "file-write", &[p("path", Kind::Path)], None,
        &[flag("append", 'a', "připojit na konec")], IA, IN,
        "uloží bajty nebo text do souboru (záznamy jen po `to text`/`to json`)"),
    sig!("rm", "file-remove", NONE, Some(Kind::Path), &[flag("recursive", 'r', "i složky s obsahem")], IN, IN,
        "smaže soubory"),
    sig!("cp", "file-copy", &[p("from", Kind::Path), p("to", Kind::Path)], None,
        &[flag("recursive", 'r', "i složky")], IN, IN, "zkopíruje soubor"),
    sig!("mv", "file-move", &[p("from", Kind::Path), p("to", Kind::Path)], None, NOFLAGS, IN, IN,
        "přesune nebo přejmenuje soubor"),
    sig!("mkdir", "dir-create", NONE, Some(Kind::Path), &[flag("parents", 'p', "vytvořit i nadřazené složky")],
        IN, IN, "vytvoří složku"),
    sig!("exists", "file-exists", &[p("path", Kind::Path)], None, NOFLAGS, IN, IA, "existuje soubor?"),
    sig!("df", "fs-info", &[opt("path", Kind::Path)], None, NOFLAGS, IN, Io::Records(&["total", "used", "free"]),
        "místo na souborovém systému"),
    // text a vystup
    sig!("echo", "text-print", NONE, Some(Kind::Any), NOFLAGS, IN, IT, "vrátí argumenty spojené mezerou"),
    sig!("log", "text-log", NONE, Some(Kind::Any), NOFLAGS, IN, IN, "vypíše poznámku do terminálu"),
    sig!("lines", "text-lines", NONE, None, NOFLAGS, IA, Io::Records(&[]), "rozdělí text na řádky"),
    sig!("split", "text-split", &[p("sep", Kind::Str)], None, NOFLAGS, IT, Io::Records(&[]), "rozdělí text"),
    sig!("join", "text-join", &[opt("sep", Kind::Str)], None, NOFLAGS, IA, IT, "spojí seznam textů"),
    sig!("trim", "text-trim", NONE, None, NOFLAGS, IT, IT, "odstraní mezery na okrajích"),
    sig!("upper", "text-upper", NONE, None, NOFLAGS, IT, IT, "velká písmena"),
    sig!("lower", "text-lower", NONE, None, NOFLAGS, IT, IT, "malá písmena"),
    sig!("contains", "text-contains", &[p("part", Kind::Any)], None, NOFLAGS, IA, IA,
        "obsahuje text (nebo seznam) danou část?"),
    sig!("starts-with", "text-starts-with", &[p("prefix", Kind::Str)], None, NOFLAGS, IT, IA, "začíná textem?"),
    sig!("ends-with", "text-ends-with", &[p("suffix", Kind::Str)], None, NOFLAGS, IT, IA, "končí textem?"),
    sig!("replace", "text-replace", &[p("from", Kind::Str), p("to", Kind::Str)], None, NOFLAGS, IT, IT,
        "nahradí všechny výskyty"),
    // seznamy a tabulky
    sig!("where", "list-filter", &[p("cond", Kind::Cond)], None, NOFLAGS, IA, IA, "nechá řádky, které splní podmínku"),
    sig!("each", "list-map", &[p("f", Kind::Closure)], None,
        &[Flag { long: "parallel", short: Some('p'), value: Some(Kind::Int), help: "nejvýš N řádků souběžně (úlohy)" }],
        IA, IA, "použije funkci na každý řádek"),
    sig!("select", "record-select", NONE, Some(Kind::Str), NOFLAGS, IA, IA, "nechá jen vybrané sloupce"),
    sig!("sort", "list-sort", &[opt("by", Kind::Str)], None, &[flag("desc", 'd', "sestupně")], IA, IA,
        "seřadí řádky (podle sloupce)"),
    sig!("first", "list-first", &[opt("n", Kind::Int)], None, NOFLAGS, IA, IA, "první řádek (nebo prvních n)"),
    sig!("last", "list-last", &[opt("n", Kind::Int)], None, NOFLAGS, IA, IA, "poslední řádek (nebo posledních n)"),
    sig!("count", "list-count", NONE, None, NOFLAGS, IA, IA, "počet řádků (nebo znaků textu)"),
    sig!("sum", "list-sum", &[opt("field", Kind::Str)], None, NOFLAGS, IA, IA, "součet (sloupce)"),
    sig!("group", "list-group", &[p("by", Kind::Str)], None, NOFLAGS, IA, Io::Records(&["key", "items"]),
        "seskupí řádky podle sloupce"),
    sig!("reverse", "list-reverse", NONE, None, NOFLAGS, IA, IA, "obrátí pořadí"),
    sig!("range", "list-range", &[p("from", Kind::Int), p("to", Kind::Int)], None, NOFLAGS, IN, IA,
        "čísla od..do (bez horní meze)"),
    // prevody (jen vyslovne)
    sig!("from", "data-from", &[p("format", Kind::Str)], None, NOFLAGS, IA, IA,
        "převede bajty/text na hodnoty (json, text, lines)"),
    sig!("to", "data-to", &[p("format", Kind::Str)], None, NOFLAGS, IA, IA, "převede hodnoty na text (json, text)"),
    sig!("describe", "value-type", NONE, None, NOFLAGS, IA, IT, "typ hodnoty"),
    sig!("convert", "value-convert", &[opt("target", Kind::Str)], None,
        &[Flag { long: "from", short: Some('f'), value: Some(Kind::Str), help: "zdrojová jednotka, soustava nebo kódování" }],
        IA, IA, "převede jednotky, číselné soustavy, kódování textu a časová pásma"),
    // cas
    sig!("now", "time-now", NONE, None, NOFLAGS, IN, IA, "aktuální čas"),
    sig!("sleep", "time-sleep", &[p("time", Kind::Duration)], None, NOFLAGS, IN, IN, "počká"),
    // zvuk
    sig!("audio", "audio-control", &[p("action", Kind::Str)], Some(Kind::Any),
        &[Flag { long: "duration", short: Some('d'), value: Some(Kind::Duration), help: "délka tónu (výchozí 1s)" },
          Flag { long: "level", short: Some('l'), value: Some(Kind::Int), help: "úroveň tónu v % (výchozí 50)" }],
        IN, IA, "zvuk: tone [Hz], play soubor.wav, volume [%], mute, unmute, status, devices, device N"),
    // relace, ulohy, bezpecnost
    sig!("undo", "command-undo", NONE, None, NOFLAGS, IN, IA, "vrátí změny posledního příkazu"),
    sig!("preview", "command-preview", &[p("body", Kind::Block)], None,
        &[flag("yes", 'y', "převzít bez dotazu")], IN, IA,
        "spustí blok nanečisto, ukáže změny a po potvrzení je převezme"),
    sig!("jobs", "job-list", NONE, None, NOFLAGS, IN, Io::Records(&["id", "state", "command"]),
        "úlohy na pozadí"),
    sig!("wait", "job-wait", &[opt("id", Kind::Int)], None, NOFLAGS, IN, IA, "počká na úlohu a vrátí její výsledek"),
    sig!("kill", "job-kill", &[p("id", Kind::Int)], None, NOFLAGS, IN, IN, "ukončí úlohu"),
    sig!("fg", "job-foreground", &[opt("id", Kind::Int)], None, NOFLAGS, IN, IN,
        "vrátí úlohu z pozadí do popředí (Ctrl+C ji přeruší, Ctrl+Z vrátí zpět)"),
    sig!("cancel", "task-cancel", &[p("task", Kind::Any)], None, NOFLAGS, IN, IN, "zruší úlohu async (nebo seznam úloh)"),
    sig!("tasks", "task-list", &[opt("action", Kind::Str), opt("id", Kind::Int)], None,
        &[Flag { long: "lines", short: Some('n'), value: Some(Kind::Int), help: "kolik řádků výstupu (tasks log)" }],
        IN, Io::Records(&["id", "name", "state", "owner", "who", "agent", "progress", "eta", "cpu", "memory", "started", "ended"]),
        "přehled úloh (vše na pozadí): tasks, tasks cancel|pause|resume|log <id>"),
    sig!("history", "history-list", NONE, None, NOFLAGS, IN,
        Io::Records(&["line", "cwd", "ok", "duration", "time"]), "historie příkazů s kontextem"),
    sig!("explain", "error-explain", &[p("code", Kind::Str)], None, NOFLAGS, IA, IT,
        "vysvětlí kód chyby nebo hlášku z logu"),
    sig!("help", "help-show", &[opt("command", Kind::Str)], None, NOFLAGS, IN, IT, "nápověda k příkazu"),
    sig!("run", "script-run", &[p("path", Kind::Path)], None, NOFLAGS, IN, IA,
        "spustí skript (kontroluje jeho manifest needs)"),
    sig!("error", "error-raise", &[p("message", Kind::Str)], None,
        &[Flag { long: "code", short: None, value: Some(Kind::Str), help: "kód chyby" }], IN, IN, "vyvolá chybu"),
    sig!("exit", "session-exit", NONE, None, NOFLAGS, IN, IN, "ukončí relaci"),
    sig!("shutdown", "system-shutdown", NONE, None, &[flag("yes", 'y', "vypnout bez dotazu")], IN, IN,
        "čistě vypne počítač (ukončí služby, pak ACPI S5)"),
    sig!("reboot", "system-reboot", NONE, None, &[flag("yes", 'y', "restartovat bez dotazu")], IN, IN,
        "čistě restartuje počítač"),
    sig!("install", "system-install", NONE, None,
        &[Flag { long: "odpovedi", short: None, value: Some(Kind::Str), help: "soubor odpovědí (instalace bez dotazů)" }], IN, IN,
        "nainstaluje systém ze živého USB na disk (průvodce v krocích; s --odpovedi bez dotazů)"),
    sig!("theme", "system-theme", &[opt("what", Kind::Str), opt("value", Kind::Str)], None,
        &[flag("system", 's', "systémová volba (přihlašovací obrazovka; potvrdí správce)")], IN,
        Io::Records(&["scheme", "contrast", "reduce_motion"]),
        "vzhled: light, dark, auto (podle slunce); contrast on|off, reduce-motion on|off; bez argumentu stav"),
    sig!("permissions", "system-permissions", &[opt("action", Kind::Str), opt("app", Kind::Str), opt("permission", Kind::Str)], None,
        NOFLAGS, IN, Io::Records(&["app", "permission", "allowed"]),
        "povolení aplikací (mikrofon): bez argumentu výpis, `permissions revoke <aplikace> [microphone]`"),
    sig!("sshd", "remote-sshd", &[opt("action", Kind::Str)], None, NOFLAGS, IN,
        Io::Records(&["enabled", "running", "port", "host_key", "sessions", "connected"]),
        "vzdálený přístup SSH pro celý počítač: bez argumentu stav, `sshd enable` / `sshd disable` (potvrdí správce)"),
    sig!("ssh-key", "remote-ssh-key", &[opt("action", Kind::Str)], Some(Kind::Any), NOFLAGS, IA,
        Io::Records(&["id", "algo", "fingerprint", "comment", "added"]),
        "klíče SSH vašeho účtu: výpis, `ssh-key add <soubor.pub|->` (chce heslo), `ssh-key remove <číslo>`"),
    sig!("remote-access", "remote-access", &[opt("state", Kind::Str)], None, NOFLAGS, IN, Io::Records(&["remote_access"]),
        "vzdálený přístup vašeho účtu: bez argumentu stav, `remote-access on` (chce heslo) / `off`"),
    sig!("acl", "file-acl", &[p("path", Kind::Any)], Some(Kind::Any),
        &[flag("inherit", 'i', "grant se dědí i do obsahu složky")], IN,
        Io::Records(&["path", "owner", "your_level", "your_label", "inherits", "grants"]),
        "oprávnění souboru: `acl <cesta>` výpis, `acl add <kdo> <view|run|edit|manage> <cesta>`, `acl remove <kdo> <cesta>`"),
    sig!("system", "system-file", &[opt("op", Kind::Str), opt("path", Kind::Path), opt("source", Kind::Path)], None, NOFLAGS, IN, IN,
        "změna v /system (úkon správy, potvrdí člověk s rolí): `system write <cíl> <zdroj>`, `system rm <cesta>`, `system mkdir <cesta>`"),
    #[cfg(feature = "dev-mode")]
    sig!("devmode", "system-dev-mode", &[opt("state", Kind::Str)], None, NOFLAGS, IN, Io::Records(&["devmode"]),
        "režim vývojáře (jen vývojové sestavení): `devmode on|off`, bez argumentu stav"),
    sig!("loglevel", "system-log-level", NONE, None, NOFLAGS, IN, IN,
        "úroveň logu: bez argumentu výpis, `loglevel <modul|proces> <error|warn|info|debug|trace|vychozi>`"),
    sig!("report", "system-report", NONE, None, NOFLAGS, IN, IN,
        "balíček pro vývojáře: logy, záznamy pádů a informace o stroji do jednoho souboru (vypíše cestu a obsah)"),
];
