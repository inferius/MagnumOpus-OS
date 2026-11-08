//! Nativni protokol terminalu (verze 1).
//!
//! Zpravy jsou binarni (little-endian): 1 B druh zpravy, pak pole. Text je
//! `u32` delka + UTF-8. Hodnoty shellu se posilaji typovane; seznam zaznamu
//! jako **tabulka se schematem** (jmena sloupcu jednou, pak radky), takze
//! terminal vi, ze `size` je velikost a `mtime` cas, a zobrazi je podle
//! sebe (zarovnani, jednotky, mistni cas). Prevod na text dela az terminal.

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;

use shlang::value::Value;

/// Verze 2: `seq` v `Complete`/`Completions` a `Format`/`Formatted`.
/// Verze 3: prehled uloh (`Tasks`, `TaskAction`).
pub const VERSION: u32 = 3;

/// Polozka vystupu v bloku.
#[derive(Debug, Clone, PartialEq)]
pub enum Item {
    /// Hodnota (tabulka, zaznam, text, cislo, ...).
    Value(Value),
    /// Poznamka (`log`, oznameni ulohy).
    Note(String),
    /// Varovani parseru (napr. "myslel jsi $x?").
    Warning(String),
    /// Chyba: kod, popis a hlaseni s mistem ve zdroji.
    Error { code: String, message: String, report: String },
    /// Prevzate zmeny souboru (tabulka `{change, path, before, after}`).
    Diff(Value),
    /// Prubeh dlouhe operace.
    Progress { done: u64, total: u64, label: String },
    /// Odkaz (soubor, napoveda).
    Link { text: String, target: String },
}

/// Terminal -> sluzba.
#[derive(Debug, Clone, PartialEq)]
pub enum ClientMsg {
    Hello { version: u32, cols: u16, rows: u16 },
    /// Pripojit k relaci (`0` = nova).
    Attach { session: u64 },
    /// Odpojit (zavreni okna); relace bezi dal.
    Detach { session: u64 },
    Run { session: u64, line: String },
    Interrupt { session: u64 },
    /// Doplneni; `seq` = cislo pozadavku (vrati se v `Completions`).
    Complete { session: u64, line: String, cursor: u32, seq: u32 },
    Answer { prompt: u64, yes: bool },
    /// Prevod radku do druheho zapisu (0 = B, 1 = C).
    Format { session: u64, line: String, style: u8, seq: u32 },
    Sessions,
    /// Odpoved na vyzvu s volbami (`index` do `Prompt::choices`).
    Choose { prompt: u64, index: u32 },
    /// Vyznamove zvyrazneni rozepsaneho radku (okno ho posila pri psani;
    /// `seq` = poradi, okno zahodi starsi odpovedi).
    Highlight { session: u64, line: String, seq: u32 },
    /// Akce nad ulohou z panelu uloh (`action`: 0 zrusit, 1 pozastavit,
    /// 2 pokracovat).
    TaskAction { session: u64, task: u64, action: u8 },
}

/// Sluzba -> terminal.
#[derive(Debug, Clone, PartialEq)]
pub enum ServerMsg {
    Welcome { version: u32 },
    Attached { session: u64, cwd: String, blocks: u32 },
    BlockStart { session: u64, block: u64, line: String },
    Item { session: u64, block: u64, item: Item },
    BlockEnd { session: u64, block: u64, ok: bool, duration: i64 },
    /// Vyzva: `choices` prazdne = ano/ne (`Answer`), jinak vyber (`Choose`).
    Prompt { prompt: u64, question: String, detail: Item, choices: Vec<String> },
    /// Zmena polozky bloku na miste (pruh prubehu, stav): `index` = poradi
    /// polozky v bloku.
    ItemUpdate { session: u64, block: u64, index: u32, item: Item },
    /// Odpoved na `Complete` se stejnym `seq` (okno ji sparuje i kdyz
    /// mezitim prisly udalosti bloku).
    Completions { seq: u32, start: u32, items: Vec<(String, u8, String)> },
    SessionList { sessions: Vec<(u64, u32, String)> },
    /// Odpoved na `Format` se stejnym `seq`.
    Formatted { seq: u32, text: String },
    Fail { message: String },
    /// Odpoved na `Highlight`: useky (zacatek, delka v bajtech, trida
    /// `shlang::highlight::Class`) a misto chyby parseru.
    Highlighted { seq: u32, spans: Vec<(u32, u32, u8)>, error: Option<(u32, u32, String)> },
    /// Ulohy uzivatele z prehledu uloh (`taskd`): cely seznam po kazde
    /// zmene (panel uloh, stavovy radek, upozorneni).
    Tasks { tasks: Vec<taskproto::Task> },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DecodeError;

// ---- zapis ----

#[derive(Default)]
pub struct W(pub Vec<u8>);

impl W {
    pub(crate) fn u8(&mut self, v: u8) {
        self.0.push(v);
    }
    pub(crate) fn u16(&mut self, v: u16) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    pub(crate) fn u32(&mut self, v: u32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    pub(crate) fn u64(&mut self, v: u64) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    pub(crate) fn i64(&mut self, v: i64) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    pub(crate) fn bytes(&mut self, b: &[u8]) {
        self.u32(b.len() as u32);
        self.0.extend_from_slice(b);
    }
    pub(crate) fn str(&mut self, s: &str) {
        self.bytes(s.as_bytes());
    }
}

pub struct R<'a> {
    b: &'a [u8],
    i: usize,
}

impl<'a> R<'a> {
    pub fn new(b: &'a [u8]) -> Self {
        Self { b, i: 0 }
    }
    pub(crate) fn take(&mut self, n: usize) -> Result<&'a [u8], DecodeError> {
        let s = self.b.get(self.i..self.i.checked_add(n).ok_or(DecodeError)?).ok_or(DecodeError)?;
        self.i += n;
        Ok(s)
    }
    pub(crate) fn u8(&mut self) -> Result<u8, DecodeError> {
        Ok(self.take(1)?[0])
    }
    pub(crate) fn u16(&mut self) -> Result<u16, DecodeError> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }
    pub(crate) fn u32(&mut self) -> Result<u32, DecodeError> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    pub(crate) fn u64(&mut self) -> Result<u64, DecodeError> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
    pub(crate) fn i64(&mut self) -> Result<i64, DecodeError> {
        Ok(i64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
    pub(crate) fn bytes(&mut self) -> Result<Vec<u8>, DecodeError> {
        let n = self.u32()? as usize;
        Ok(self.take(n)?.to_vec())
    }
    pub(crate) fn str(&mut self) -> Result<String, DecodeError> {
        String::from_utf8(self.bytes()?).map_err(|_| DecodeError)
    }
    pub(crate) fn done(&self) -> Result<(), DecodeError> {
        if self.i == self.b.len() { Ok(()) } else { Err(DecodeError) }
    }
}

// ---- hodnoty ----

const V_NOTHING: u8 = 0;
const V_NULL: u8 = 1;
const V_BOOL: u8 = 2;
const V_INT: u8 = 3;
const V_FLOAT: u8 = 4;
const V_SIZE: u8 = 5;
const V_DURATION: u8 = 6;
const V_TIME: u8 = 7;
const V_STR: u8 = 8;
const V_BYTES: u8 = 9;
const V_LIST: u8 = 10;
const V_RECORD: u8 = 11;
const V_ERROR: u8 = 12;
const V_TABLE: u8 = 13;

/// Je seznam tabulka (vsechny radky jsou zaznamy se stejnymi sloupci)?
fn table_cols(items: &[Value]) -> Option<Vec<String>> {
    let Some(Value::Record(first)) = items.first() else { return None };
    let cols: Vec<String> = first.iter().map(|(k, _)| k.clone()).collect();
    let same = items.iter().all(|r| match r {
        Value::Record(f) => f.len() == cols.len() && f.iter().zip(&cols).all(|((k, _), c)| k == c),
        _ => false,
    });
    same.then_some(cols)
}

pub fn put_value(w: &mut W, v: &Value) {
    match v {
        Value::Nothing => w.u8(V_NOTHING),
        Value::Null => w.u8(V_NULL),
        Value::Bool(b) => {
            w.u8(V_BOOL);
            w.u8(*b as u8);
        }
        Value::Int(i) => {
            w.u8(V_INT);
            w.i64(*i);
        }
        Value::Float(f) => {
            w.u8(V_FLOAT);
            w.u64(f.to_bits());
        }
        Value::Size(i) => {
            w.u8(V_SIZE);
            w.i64(*i);
        }
        Value::Duration(i) => {
            w.u8(V_DURATION);
            w.i64(*i);
        }
        Value::Time(i) => {
            w.u8(V_TIME);
            w.i64(*i);
        }
        Value::Str(s) => {
            w.u8(V_STR);
            w.str(s);
        }
        Value::Bytes(b) => {
            w.u8(V_BYTES);
            w.bytes(b);
        }
        Value::List(items) => {
            if let Some(cols) = table_cols(items) {
                w.u8(V_TABLE);
                w.u32(cols.len() as u32);
                for c in &cols {
                    w.str(c);
                }
                w.u32(items.len() as u32);
                for r in items {
                    if let Value::Record(f) = r {
                        for (_, x) in f {
                            put_value(w, x);
                        }
                    }
                }
            } else {
                w.u8(V_LIST);
                w.u32(items.len() as u32);
                for x in items {
                    put_value(w, x);
                }
            }
        }
        Value::Record(f) => {
            w.u8(V_RECORD);
            w.u32(f.len() as u32);
            for (k, x) in f {
                w.str(k);
                put_value(w, x);
            }
        }
        Value::Error(e) => {
            w.u8(V_ERROR);
            w.str(&e.code);
            w.str(&e.message);
        }
        Value::Closure(_) => {
            w.u8(V_STR);
            w.str(&v.to_text());
        }
    }
}

const MAX_ITEMS: usize = 1 << 20;

pub fn get_value(r: &mut R, depth: usize) -> Result<Value, DecodeError> {
    if depth > 64 {
        return Err(DecodeError);
    }
    Ok(match r.u8()? {
        V_NOTHING => Value::Nothing,
        V_NULL => Value::Null,
        V_BOOL => Value::Bool(r.u8()? != 0),
        V_INT => Value::Int(r.i64()?),
        V_FLOAT => Value::Float(f64::from_bits(r.u64()?)),
        V_SIZE => Value::Size(r.i64()?),
        V_DURATION => Value::Duration(r.i64()?),
        V_TIME => Value::Time(r.i64()?),
        V_STR => Value::Str(r.str()?),
        V_BYTES => Value::Bytes(r.bytes()?),
        V_LIST => {
            let n = r.u32()? as usize;
            if n > MAX_ITEMS {
                return Err(DecodeError);
            }
            let mut v = Vec::new();
            for _ in 0..n {
                v.push(get_value(r, depth + 1)?);
            }
            Value::List(v)
        }
        V_RECORD => {
            let n = r.u32()? as usize;
            if n > MAX_ITEMS {
                return Err(DecodeError);
            }
            let mut f = Vec::new();
            for _ in 0..n {
                let k = r.str()?;
                f.push((k, get_value(r, depth + 1)?));
            }
            Value::Record(f)
        }
        V_TABLE => {
            let nc = r.u32()? as usize;
            if nc == 0 || nc > 4096 {
                return Err(DecodeError);
            }
            let mut cols = Vec::new();
            let mut names = 0usize;
            for _ in 0..nc {
                let c = r.str()?;
                names += c.len();
                cols.push(c);
            }
            let nr = r.u32()? as usize;
            // Jmena sloupcu se kopiruji do kazdeho radku: omezit, kolik to
            // stoji (kazda bunka stoji aspon 1 B vstupu, jmena ne).
            if nr > MAX_ITEMS || nr.saturating_mul(nc) > MAX_ITEMS || nr.saturating_mul(names) > 64 << 20 {
                return Err(DecodeError);
            }
            let mut rows = Vec::new();
            for _ in 0..nr {
                let mut f = Vec::new();
                for c in &cols {
                    f.push((c.clone(), get_value(r, depth + 1)?));
                }
                rows.push(Value::Record(f));
            }
            Value::List(rows)
        }
        V_ERROR => {
            let code = r.str()?;
            let msg = r.str()?;
            Value::Error(Box::new(shlang::ShError::new(&code, msg)))
        }
        _ => return Err(DecodeError),
    })
}

fn put_item(w: &mut W, it: &Item) {
    match it {
        Item::Value(v) => {
            w.u8(0);
            put_value(w, v);
        }
        Item::Note(s) => {
            w.u8(1);
            w.str(s);
        }
        Item::Warning(s) => {
            w.u8(2);
            w.str(s);
        }
        Item::Error { code, message, report } => {
            w.u8(3);
            w.str(code);
            w.str(message);
            w.str(report);
        }
        Item::Diff(v) => {
            w.u8(4);
            put_value(w, v);
        }
        Item::Progress { done, total, label } => {
            w.u8(5);
            w.u64(*done);
            w.u64(*total);
            w.str(label);
        }
        Item::Link { text, target } => {
            w.u8(6);
            w.str(text);
            w.str(target);
        }
    }
}

fn get_item(r: &mut R) -> Result<Item, DecodeError> {
    Ok(match r.u8()? {
        0 => Item::Value(get_value(r, 0)?),
        1 => Item::Note(r.str()?),
        2 => Item::Warning(r.str()?),
        3 => Item::Error { code: r.str()?, message: r.str()?, report: r.str()? },
        4 => Item::Diff(get_value(r, 0)?),
        5 => Item::Progress { done: r.u64()?, total: r.u64()?, label: r.str()? },
        6 => Item::Link { text: r.str()?, target: r.str()? },
        _ => return Err(DecodeError),
    })
}

impl ClientMsg {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = W::default();
        match self {
            ClientMsg::Hello { version, cols, rows } => {
                w.u8(1);
                w.u32(*version);
                w.u16(*cols);
                w.u16(*rows);
            }
            ClientMsg::Attach { session } => {
                w.u8(2);
                w.u64(*session);
            }
            ClientMsg::Detach { session } => {
                w.u8(3);
                w.u64(*session);
            }
            ClientMsg::Run { session, line } => {
                w.u8(4);
                w.u64(*session);
                w.str(line);
            }
            ClientMsg::Interrupt { session } => {
                w.u8(5);
                w.u64(*session);
            }
            ClientMsg::Complete { session, line, cursor, seq } => {
                w.u8(6);
                w.u64(*session);
                w.str(line);
                w.u32(*cursor);
                w.u32(*seq);
            }
            ClientMsg::Answer { prompt, yes } => {
                w.u8(7);
                w.u64(*prompt);
                w.u8(*yes as u8);
            }
            ClientMsg::Format { session, line, style, seq } => {
                w.u8(8);
                w.u64(*session);
                w.str(line);
                w.u8(*style);
                w.u32(*seq);
            }
            ClientMsg::Sessions => w.u8(9),
            ClientMsg::Choose { prompt, index } => {
                w.u8(11);
                w.u64(*prompt);
                w.u32(*index);
            }
            ClientMsg::Highlight { session, line, seq } => {
                w.u8(10);
                w.u64(*session);
                w.str(line);
                w.u32(*seq);
            }
            ClientMsg::TaskAction { session, task, action } => {
                w.u8(12);
                w.u64(*session);
                w.u64(*task);
                w.u8(*action);
            }
        }
        w.0
    }

    pub fn decode(b: &[u8]) -> Result<Self, DecodeError> {
        let mut r = R::new(b);
        let m = match r.u8()? {
            1 => ClientMsg::Hello { version: r.u32()?, cols: r.u16()?, rows: r.u16()? },
            2 => ClientMsg::Attach { session: r.u64()? },
            3 => ClientMsg::Detach { session: r.u64()? },
            4 => ClientMsg::Run { session: r.u64()?, line: r.str()? },
            5 => ClientMsg::Interrupt { session: r.u64()? },
            6 => ClientMsg::Complete { session: r.u64()?, line: r.str()?, cursor: r.u32()?, seq: r.u32()? },
            7 => ClientMsg::Answer { prompt: r.u64()?, yes: r.u8()? != 0 },
            8 => ClientMsg::Format { session: r.u64()?, line: r.str()?, style: r.u8()?, seq: r.u32()? },
            9 => ClientMsg::Sessions,
            10 => ClientMsg::Highlight { session: r.u64()?, line: r.str()?, seq: r.u32()? },
            11 => ClientMsg::Choose { prompt: r.u64()?, index: r.u32()? },
            12 => ClientMsg::TaskAction { session: r.u64()?, task: r.u64()?, action: r.u8()? },
            _ => return Err(DecodeError),
        };
        r.done()?;
        Ok(m)
    }
}

impl ServerMsg {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = W::default();
        match self {
            ServerMsg::Welcome { version } => {
                w.u8(1);
                w.u32(*version);
            }
            ServerMsg::Attached { session, cwd, blocks } => {
                w.u8(2);
                w.u64(*session);
                w.str(cwd);
                w.u32(*blocks);
            }
            ServerMsg::BlockStart { session, block, line } => {
                w.u8(3);
                w.u64(*session);
                w.u64(*block);
                w.str(line);
            }
            ServerMsg::Item { session, block, item } => {
                w.u8(4);
                w.u64(*session);
                w.u64(*block);
                put_item(&mut w, item);
            }
            ServerMsg::BlockEnd { session, block, ok, duration } => {
                w.u8(5);
                w.u64(*session);
                w.u64(*block);
                w.u8(*ok as u8);
                w.i64(*duration);
            }
            ServerMsg::Prompt { prompt, question, detail, choices } => {
                w.u8(6);
                w.u64(*prompt);
                w.str(question);
                put_item(&mut w, detail);
                w.u32(choices.len() as u32);
                for c in choices {
                    w.str(c);
                }
            }
            ServerMsg::ItemUpdate { session, block, index, item } => {
                w.u8(12);
                w.u64(*session);
                w.u64(*block);
                w.u32(*index);
                put_item(&mut w, item);
            }
            ServerMsg::Completions { seq, start, items } => {
                w.u8(7);
                w.u32(*seq);
                w.u32(*start);
                w.u32(items.len() as u32);
                for (t, k, h) in items {
                    w.str(t);
                    w.u8(*k);
                    w.str(h);
                }
            }
            ServerMsg::SessionList { sessions } => {
                w.u8(8);
                w.u32(sessions.len() as u32);
                for (id, n, cwd) in sessions {
                    w.u64(*id);
                    w.u32(*n);
                    w.str(cwd);
                }
            }
            ServerMsg::Formatted { seq, text } => {
                w.u8(9);
                w.u32(*seq);
                w.str(text);
            }
            ServerMsg::Fail { message } => {
                w.u8(10);
                w.str(message);
            }
            ServerMsg::Tasks { tasks } => {
                w.u8(13);
                let snap = taskproto::Reply::Snapshot { req: 0, seq: 0, tasks: tasks.clone(), last: true };
                w.bytes(&snap.encode());
            }
            ServerMsg::Highlighted { seq, spans, error } => {
                w.u8(11);
                w.u32(*seq);
                w.u32(spans.len() as u32);
                for (a, b, c) in spans {
                    w.u32(*a);
                    w.u32(*b);
                    w.u8(*c);
                }
                match error {
                    None => w.u8(0),
                    Some((a, b, m)) => {
                        w.u8(1);
                        w.u32(*a);
                        w.u32(*b);
                        w.str(m);
                    }
                }
            }
        }
        w.0
    }

    pub fn decode(b: &[u8]) -> Result<Self, DecodeError> {
        let mut r = R::new(b);
        let m = match r.u8()? {
            1 => ServerMsg::Welcome { version: r.u32()? },
            2 => ServerMsg::Attached { session: r.u64()?, cwd: r.str()?, blocks: r.u32()? },
            3 => ServerMsg::BlockStart { session: r.u64()?, block: r.u64()?, line: r.str()? },
            4 => ServerMsg::Item { session: r.u64()?, block: r.u64()?, item: get_item(&mut r)? },
            5 => ServerMsg::BlockEnd { session: r.u64()?, block: r.u64()?, ok: r.u8()? != 0, duration: r.i64()? },
            6 => {
                let (prompt, question, detail) = (r.u64()?, r.str()?, get_item(&mut r)?);
                let n = r.u32()? as usize;
                if n > MAX_ITEMS {
                    return Err(DecodeError);
                }
                let mut choices = Vec::new();
                for _ in 0..n {
                    choices.push(r.str()?);
                }
                ServerMsg::Prompt { prompt, question, detail, choices }
            }
            13 => match taskproto::Reply::decode(&r.bytes()?) {
                Ok(taskproto::Reply::Snapshot { tasks, .. }) => ServerMsg::Tasks { tasks },
                _ => return Err(DecodeError),
            },
            12 => ServerMsg::ItemUpdate { session: r.u64()?, block: r.u64()?, index: r.u32()?, item: get_item(&mut r)? },
            7 => {
                let seq = r.u32()?;
                let start = r.u32()?;
                let n = r.u32()? as usize;
                if n > MAX_ITEMS {
                    return Err(DecodeError);
                }
                let mut items = Vec::new();
                for _ in 0..n {
                    items.push((r.str()?, r.u8()?, r.str()?));
                }
                ServerMsg::Completions { seq, start, items }
            }
            8 => {
                let n = r.u32()? as usize;
                if n > MAX_ITEMS {
                    return Err(DecodeError);
                }
                let mut sessions = Vec::new();
                for _ in 0..n {
                    sessions.push((r.u64()?, r.u32()?, r.str()?));
                }
                ServerMsg::SessionList { sessions }
            }
            9 => ServerMsg::Formatted { seq: r.u32()?, text: r.str()? },
            10 => ServerMsg::Fail { message: r.str()? },
            11 => {
                let seq = r.u32()?;
                let n = r.u32()? as usize;
                if n > MAX_ITEMS {
                    return Err(DecodeError);
                }
                let mut spans = Vec::with_capacity(n.min(1024));
                for _ in 0..n {
                    spans.push((r.u32()?, r.u32()?, r.u8()?));
                }
                let error = match r.u8()? {
                    0 => None,
                    _ => Some((r.u32()?, r.u32()?, r.str()?)),
                };
                ServerMsg::Highlighted { seq, spans, error }
            }
            _ => return Err(DecodeError),
        };
        r.done()?;
        Ok(m)
    }
}
