//! Ridici prikazy kompozitoru (bootstrap kanal `compd` od initu)
//! a protokoly capability, ktere kompozitor rozdava: systemove dialogy,
//! volani akci a inspekce (testy, pozdeji ctecka obrazovky).
//!
//! Ridici prikaz je text (prvni slovo = jmeno), muze nest handly. Na kazdy
//! prijde jedna odpoved `ok`, `ok <data>` nebo `err <KOD>`, stejne jako
//! u `inputd`.
//!
//! | Prikaz | Handly | Odpoved |
//! |---|---|---|
//! | `display` | kanal ovladace (protokol [`crate::display`]) | `ok <w> <h>` |
//! | `input` | kanal fokusu od `inputd` | `ok` |
//! | `secure` | kanal bezpecne zkratky od `inputd` | `ok` |
//! | `connect` + [`ConnectInfo`] | - | `ok` + kanal okna |
//! | `dialog` | - | `ok` + kanal dialogu ([`DialogRequest`]) |
//! | `actions <volajici>` | - | `ok` + kanal akci ([`CallerMsg`]) |
//! | `inspect` | - | `ok` + kanal inspekce ([`InspectRequest`]) |
//! | `status` | - | `ok` + [`Status`] v textu |
//! | `tasks <bezi> <agent>` | - | `ok` (ikona uloh v panelu) |
//! | `mode <w> <h> [mHz]` | - | `ok <w> <h> <mHz>` (zmena rezimu displeje) |
//! | `outputs` | - | `ok` + radek na konektor: `<vystup> <typ> <pripojen 0/1> <WxH@mHz> <rezimu> <monitor>` |

use alloc::string::String;
use alloc::vec::Vec;

use crate::a11y::Node;
use crate::action::{self, Action, Arg, Value};
use crate::{Error, ReadExt, Reader, Result, WriteExt, Writer, read_count};

pub const DISPLAY: &str = "display";
pub const INPUT: &str = "input";
pub const SECURE: &str = "secure";
pub const CONNECT: &str = "connect";
pub const DIALOG: &str = "dialog";
pub const ACTIONS: &str = "actions";
pub const INSPECT: &str = "inspect";
pub const STATUS: &str = "status";
/// `tasks <bezi> <z toho agent>`: pocet uloh pro ikonu v panelu (do
/// pripojeni kompozitoru k taskd ho posila init nebo test).
pub const TASKS: &str = "tasks";
/// `mode <w> <h> [obnova mHz]`: zmena rezimu displeje (`SET_MODE`).
pub const MODE: &str = "mode";
/// `outputs`: konektory a rezimy displeje (`CONNECTORS`) jako text.
pub const OUTPUTS: &str = "outputs";

/// Identita aplikace pri `connect`: posila ji init (duveryhodny), ne
/// aplikace. Odznak duvery ukazuje `name` a `perms`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectInfo {
    /// Id z manifestu (`org.magnumopus.demo`).
    pub id: String,
    /// Jmeno pro cloveka.
    pub name: String,
    /// Co aplikace smi (kratke stitky: `sit`, `soubory`, `kamera`).
    pub perms: Vec<String>,
    /// Akce z manifestu.
    pub actions: Vec<Action>,
}

impl ConnectInfo {
    /// Zprava `connect` (prikaz + binarni data za `\n`).
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        w.bytes(CONNECT.as_bytes()).u8(b'\n');
        w.str(&self.id).str(&self.name).u32(self.perms.len() as u32);
        for p in &self.perms {
            w.str(p);
        }
        w.u32(self.actions.len() as u32);
        for a in &self.actions {
            a.encode(&mut w);
        }
        w.finish()
    }

    /// Rozebere data za `connect\n`.
    pub fn decode(data: &[u8]) -> Result<Self> {
        let rest = data.strip_prefix(b"connect\n").ok_or(Error::InvalidArgs)?;
        let len = rest.len();
        let mut r = Reader::new(rest);
        let id = r.str()?;
        let name = r.str()?;
        let n = read_count(&mut r, 2, len)?;
        let mut perms = Vec::with_capacity(n);
        for _ in 0..n {
            perms.push(r.str()?);
        }
        let n = read_count(&mut r, 8, len)?;
        let mut actions = Vec::with_capacity(n);
        for _ in 0..n.min(action::MAX_ACTIONS + 1) {
            actions.push(Action::decode(&mut r, len)?);
        }
        if n > action::MAX_ACTIONS || !r.is_empty() {
            return Err(Error::InvalidArgs);
        }
        Ok(Self { id, name, perms, actions })
    }
}

/// Odpoved na `status`: `ok clients=1 windows=2 frames=10 presented=9
/// dialogs=0 secure=0`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Status {
    pub clients: u32,
    pub windows: u32,
    pub frames: u64,
    pub presented: u64,
    pub dialogs: u32,
    pub secure: bool,
}

impl Status {
    pub fn format(&self, out: &mut impl core::fmt::Write) -> core::fmt::Result {
        write!(
            out,
            "clients={} windows={} frames={} presented={} dialogs={} secure={}",
            self.clients, self.windows, self.frames, self.presented, self.dialogs, self.secure as u8
        )
    }

    pub fn parse(text: &str) -> Option<Self> {
        let text = text.strip_prefix("ok").unwrap_or(text);
        let mut s = Status::default();
        for part in text.split_whitespace() {
            let (k, v) = part.split_once('=')?;
            match k {
                "clients" => s.clients = v.parse().ok()?,
                "windows" => s.windows = v.parse().ok()?,
                "frames" => s.frames = v.parse().ok()?,
                "presented" => s.presented = v.parse().ok()?,
                "dialogs" => s.dialogs = v.parse().ok()?,
                "secure" => s.secure = v == "1",
                _ => {}
            }
        }
        Some(s)
    }
}

/// Druh systemoveho dialogu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DialogKind {
    /// Povolit opravneni (volby jako tlacitka).
    Permission,
    /// Potvrdit akci (agent, `confirm = true`).
    Confirm,
    /// Zadat heslo (odpoved je text).
    Password,
}

impl DialogKind {
    const fn to_u8(self) -> u8 {
        match self {
            DialogKind::Permission => 0,
            DialogKind::Confirm => 1,
            DialogKind::Password => 2,
        }
    }

    fn from_u8(v: u8) -> Result<Self> {
        Ok(match v {
            0 => DialogKind::Permission,
            1 => DialogKind::Confirm,
            2 => DialogKind::Password,
            _ => return Err(Error::InvalidArgs),
        })
    }
}

/// Kanal `dialog`: zadost (jen sluzba s touto capability, nikdy aplikace).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DialogRequest {
    Ask {
        id: u32,
        kind: DialogKind,
        /// Jmeno aplikace, ktera o neco zada (z jeji identity).
        app: String,
        text: String,
        /// Volby (tlacitka); prvni je vychozi pro Enter, posledni pro Esc.
        options: Vec<String>,
    },
    /// Zrusit dialog (zadost uz neplati).
    Cancel { id: u32 },
    /// Nastavit bezpecnostni frazi uzivatele (zobrazi se v kazdem dialogu).
    Phrase { text: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DialogReply {
    /// Uzivatel zvolil `choice` (index do voleb) nebo zadal `text` (heslo).
    Answer { id: u32, choice: u32, text: String },
    /// Dialog zrusen (Cancel, zavreni kanalu).
    Cancelled { id: u32 },
}

impl DialogRequest {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        match self {
            DialogRequest::Ask { id, kind, app, text, options } => {
                w.u8(1).u32(*id).u8(kind.to_u8()).str(app).str(text).u32(options.len() as u32);
                for o in options {
                    w.str(o);
                }
            }
            DialogRequest::Cancel { id } => {
                w.u8(2).u32(*id);
            }
            DialogRequest::Phrase { text } => {
                w.u8(3).str(text);
            }
        }
        w.finish()
    }

    pub fn decode(data: &[u8]) -> Result<Self> {
        let mut r = Reader::new(data);
        let m = match r.u8()? {
            1 => {
                let id = r.u32()?;
                let kind = DialogKind::from_u8(r.u8()?)?;
                let app = r.str()?;
                let text = r.str()?;
                let n = read_count(&mut r, 2, data.len())?;
                if n > 8 {
                    return Err(Error::InvalidArgs);
                }
                let mut options = Vec::with_capacity(n);
                for _ in 0..n {
                    options.push(r.str()?);
                }
                if options.is_empty() {
                    return Err(Error::InvalidArgs);
                }
                DialogRequest::Ask { id, kind, app, text, options }
            }
            2 => DialogRequest::Cancel { id: r.u32()? },
            3 => DialogRequest::Phrase { text: r.str()? },
            _ => return Err(Error::NotSupported),
        };
        if !r.is_empty() {
            return Err(Error::InvalidArgs);
        }
        Ok(m)
    }
}

impl DialogReply {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        match self {
            DialogReply::Answer { id, choice, text } => {
                w.u8(1).u32(*id).u32(*choice).str(text);
            }
            DialogReply::Cancelled { id } => {
                w.u8(2).u32(*id);
            }
        }
        w.finish()
    }

    pub fn decode(data: &[u8]) -> Result<Self> {
        let mut r = Reader::new(data);
        let m = match r.u8()? {
            1 => DialogReply::Answer {
                id: r.u32()?,
                choice: r.u32()?,
                text: r.str()?,
            },
            2 => DialogReply::Cancelled { id: r.u32()? },
            _ => return Err(Error::NotSupported),
        };
        if !r.is_empty() {
            return Err(Error::InvalidArgs);
        }
        Ok(m)
    }
}

/// Kanal `actions`: volajici (shell, pruzkumnik, asistent, agent).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CallerMsg {
    /// Seznam akci bezicich aplikaci.
    List,
    /// Zavolat akci; `synthetic` = vola automat (agent) - akce se pak
    /// pred predanim potvrdi dialogem.
    Call { call: u32, app: String, action: String, args: Vec<Arg>, synthetic: bool },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CallerReply {
    Actions { list: Vec<(String, Action)> },
    Result { call: u32, ok: bool, value: Option<Value>, message: String },
}

impl CallerMsg {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        match self {
            CallerMsg::List => {
                w.u8(1);
            }
            CallerMsg::Call {
                call,
                app,
                action: a,
                args,
                synthetic,
            } => {
                w.u8(2).u32(*call).str(app).str(a).bool(*synthetic);
                action::encode_args(&mut w, args);
            }
        }
        w.finish()
    }

    pub fn decode(data: &[u8]) -> Result<Self> {
        let mut r = Reader::new(data);
        let m = match r.u8()? {
            1 => CallerMsg::List,
            2 => {
                let call = r.u32()?;
                let app = r.str()?;
                let a = r.str()?;
                let synthetic = r.bool()?;
                let args = action::decode_args(&mut r, data.len())?;
                CallerMsg::Call {
                    call,
                    app,
                    action: a,
                    args,
                    synthetic,
                }
            }
            _ => return Err(Error::NotSupported),
        };
        if !r.is_empty() {
            return Err(Error::InvalidArgs);
        }
        Ok(m)
    }
}

impl CallerReply {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        match self {
            CallerReply::Actions { list } => {
                w.u8(1).u32(list.len() as u32);
                for (app, a) in list {
                    w.str(app);
                    a.encode(&mut w);
                }
            }
            CallerReply::Result { call, ok, value, message } => {
                w.u8(2).u32(*call).bool(*ok).str(message).bool(value.is_some());
                if let Some(v) = value {
                    v.encode(&mut w);
                }
            }
        }
        w.finish()
    }

    pub fn decode(data: &[u8]) -> Result<Self> {
        let mut r = Reader::new(data);
        let m = match r.u8()? {
            1 => {
                let n = read_count(&mut r, 10, data.len())?;
                let mut list = Vec::with_capacity(n);
                for _ in 0..n {
                    let app = r.str()?;
                    list.push((app, Action::decode(&mut r, data.len())?));
                }
                CallerReply::Actions { list }
            }
            2 => {
                let call = r.u32()?;
                let ok = r.bool()?;
                let message = r.str()?;
                let value = if r.bool()? { Some(Value::decode(&mut r)?) } else { None };
                CallerReply::Result { call, ok, value, message }
            }
            _ => return Err(Error::NotSupported),
        };
        if !r.is_empty() {
            return Err(Error::InvalidArgs);
        }
        Ok(m)
    }
}

/// Kanal `inspect` (testy, pozdeji ctecka obrazovky a lupa): cteni
/// stromu pristupnosti, akce pres uzel a cteni pixelu obrazovky.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InspectRequest {
    /// Okna (id kompozitoru, aplikace, titulek, poloha).
    Windows,
    /// Strom pristupnosti okna.
    Tree { window: u32 },
    /// Akce pres uzel (jako ctecka obrazovky).
    Act { window: u32, node: u32, action: u32, value: String },
    /// Barva pixelu obrazovky (0xAARRGGBB) po poslednim slozeni.
    Pixel { x: u32, y: u32 },
    /// Pockat, az bude vse platne slozene a zobrazene.
    Settle,
}

/// Okno v odpovedi `Windows`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowInfo {
    /// Id okna v kompozitoru (globalni).
    pub id: u32,
    pub app: String,
    pub title: String,
    /// Obdelnik obsahu na obrazovce.
    pub rect: crate::Rect,
    pub focused: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InspectReply {
    Windows(Vec<WindowInfo>),
    Tree(Vec<Node>),
    Pixel(u32),
    Done,
    Error(Error),
}

impl InspectRequest {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        match self {
            InspectRequest::Windows => {
                w.u8(1);
            }
            InspectRequest::Tree { window } => {
                w.u8(2).u32(*window);
            }
            InspectRequest::Act { window, node, action, value } => {
                w.u8(3).u32(*window).u32(*node).u32(*action).str(value);
            }
            InspectRequest::Pixel { x, y } => {
                w.u8(4).u32(*x).u32(*y);
            }
            InspectRequest::Settle => {
                w.u8(5);
            }
        }
        w.finish()
    }

    pub fn decode(data: &[u8]) -> Result<Self> {
        let mut r = Reader::new(data);
        let m = match r.u8()? {
            1 => InspectRequest::Windows,
            2 => InspectRequest::Tree { window: r.u32()? },
            3 => InspectRequest::Act {
                window: r.u32()?,
                node: r.u32()?,
                action: r.u32()?,
                value: r.str()?,
            },
            4 => InspectRequest::Pixel { x: r.u32()?, y: r.u32()? },
            5 => InspectRequest::Settle,
            _ => return Err(Error::NotSupported),
        };
        if !r.is_empty() {
            return Err(Error::InvalidArgs);
        }
        Ok(m)
    }
}

impl InspectReply {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        match self {
            InspectReply::Windows(list) => {
                w.u8(1).u32(list.len() as u32);
                for i in list {
                    w.u32(i.id).str(&i.app).str(&i.title).rect(&i.rect).bool(i.focused);
                }
            }
            InspectReply::Tree(nodes) => {
                w.u8(2).u32(nodes.len() as u32);
                for n in nodes {
                    n.encode(&mut w);
                }
            }
            InspectReply::Pixel(p) => {
                w.u8(3).u32(*p);
            }
            InspectReply::Done => {
                w.u8(4);
            }
            InspectReply::Error(e) => {
                w.u8(5).i32(e.code());
            }
        }
        w.finish()
    }

    pub fn decode(data: &[u8]) -> Result<Self> {
        let mut r = Reader::new(data);
        let len = data.len();
        let m = match r.u8()? {
            1 => {
                let n = read_count(&mut r, 25, len)?;
                let mut list = Vec::with_capacity(n);
                for _ in 0..n {
                    list.push(WindowInfo {
                        id: r.u32()?,
                        app: r.str()?,
                        title: r.str()?,
                        rect: r.rect()?,
                        focused: r.bool()?,
                    });
                }
                InspectReply::Windows(list)
            }
            2 => {
                let n = read_count(&mut r, Node::MIN_SIZE, len)?;
                let mut nodes = Vec::with_capacity(n);
                for _ in 0..n {
                    nodes.push(Node::decode(&mut r)?);
                }
                InspectReply::Tree(nodes)
            }
            3 => InspectReply::Pixel(r.u32()?),
            4 => InspectReply::Done,
            5 => InspectReply::Error(Error::from_code(r.i32()? as i64).ok_or(Error::InvalidArgs)?),
            _ => return Err(Error::NotSupported),
        };
        if !r.is_empty() {
            return Err(Error::InvalidArgs);
        }
        Ok(m)
    }
}
