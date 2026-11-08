//! Podmnozina CBOR (RFC 8949) pro CTAP2: cela cisla, bajty, text, pole,
//! mapy, bool, null. Kodovani je kanonicke podle CTAP2 (klice map
//! serazene podle delky a pak bajtu zakodovaneho klice). Dekoder odmita
//! neurcite delky, tagy, cisla s plovouci carkou a vnoreni hlubsi nez
//! [`MAX_DEPTH`]; nikdy nepanikari.

use alloc::string::String;
use alloc::vec::Vec;

/// Nejhlubsi vnoreni (CTAP2 potrebuje 4).
pub const MAX_DEPTH: usize = 8;
/// Nejvic polozek v poli nebo mape.
const MAX_ITEMS: u64 = 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    Uint(u64),
    /// Zaporne cislo `-1 - n`.
    Nint(u64),
    Bytes(Vec<u8>),
    Text(String),
    Array(Vec<Value>),
    Map(Vec<(Value, Value)>),
    Bool(bool),
    Null,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CborError;

impl Value {
    pub fn int(v: i64) -> Value {
        if v >= 0 { Value::Uint(v as u64) } else { Value::Nint((-1 - v) as u64) }
    }

    pub fn text(s: &str) -> Value {
        Value::Text(String::from(s))
    }

    pub fn bytes(b: &[u8]) -> Value {
        Value::Bytes(b.to_vec())
    }

    pub fn as_int(&self) -> Option<i64> {
        match *self {
            Value::Uint(v) if v <= i64::MAX as u64 => Some(v as i64),
            Value::Nint(v) if v < i64::MAX as u64 => Some(-1 - v as i64),
            _ => None,
        }
    }

    pub fn as_bytes(&self) -> Option<&[u8]> {
        match self {
            Value::Bytes(b) => Some(b),
            _ => None,
        }
    }

    pub fn as_text(&self) -> Option<&str> {
        match self {
            Value::Text(t) => Some(t),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&[Value]> {
        match self {
            Value::Array(a) => Some(a),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }

    /// Hodnota v mape pod klicem.
    pub fn get(&self, key: &Value) -> Option<&Value> {
        match self {
            Value::Map(m) => m.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    /// Hodnota v mape pod celociselnym klicem.
    pub fn get_int(&self, key: i64) -> Option<&Value> {
        self.get(&Value::int(key))
    }

    /// Hodnota v mape pod textovym klicem.
    pub fn get_text(&self, key: &str) -> Option<&Value> {
        match self {
            Value::Map(m) => m.iter().find(|(k, _)| k.as_text() == Some(key)).map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        self.write(&mut out);
        out
    }

    fn write(&self, out: &mut Vec<u8>) {
        match self {
            Value::Uint(v) => head(out, 0, *v),
            Value::Nint(v) => head(out, 1, *v),
            Value::Bytes(b) => {
                head(out, 2, b.len() as u64);
                out.extend_from_slice(b);
            }
            Value::Text(t) => {
                head(out, 3, t.len() as u64);
                out.extend_from_slice(t.as_bytes());
            }
            Value::Array(a) => {
                head(out, 4, a.len() as u64);
                for v in a {
                    v.write(out);
                }
            }
            Value::Map(m) => {
                // Kanonicke poradi CTAP2: kratsi zakodovany klic drive,
                // pri stejne delce podle bajtu.
                let mut items: Vec<(Vec<u8>, &Value)> = m.iter().map(|(k, v)| (k.encode(), v)).collect();
                items.sort_by(|a, b| a.0.len().cmp(&b.0.len()).then_with(|| a.0.cmp(&b.0)));
                head(out, 5, items.len() as u64);
                for (k, v) in items {
                    out.extend_from_slice(&k);
                    v.write(out);
                }
            }
            Value::Bool(false) => out.push(0xF4),
            Value::Bool(true) => out.push(0xF5),
            Value::Null => out.push(0xF6),
        }
    }
}

fn head(out: &mut Vec<u8>, major: u8, v: u64) {
    let m = major << 5;
    if v < 24 {
        out.push(m | v as u8);
    } else if v <= u8::MAX as u64 {
        out.extend_from_slice(&[m | 24, v as u8]);
    } else if v <= u16::MAX as u64 {
        out.push(m | 25);
        out.extend_from_slice(&(v as u16).to_be_bytes());
    } else if v <= u32::MAX as u64 {
        out.push(m | 26);
        out.extend_from_slice(&(v as u32).to_be_bytes());
    } else {
        out.push(m | 27);
        out.extend_from_slice(&v.to_be_bytes());
    }
}

/// Dekoduje celou hodnotu (za ni nesmi nic zbyt).
pub fn decode(b: &[u8]) -> Result<Value, CborError> {
    let (v, n) = decode_prefix(b)?;
    if n != b.len() {
        return Err(CborError);
    }
    Ok(v)
}

/// Dekoduje hodnotu na zacatku `b`; vrati ji a pocet prectenych bajtu
/// (authData ma za klicem COSE dalsi data).
pub fn decode_prefix(b: &[u8]) -> Result<(Value, usize), CborError> {
    let mut p = Parser { b, i: 0 };
    let v = p.value(0)?;
    Ok((v, p.i))
}

struct Parser<'a> {
    b: &'a [u8],
    i: usize,
}

impl Parser<'_> {
    fn take(&mut self, n: usize) -> Result<&[u8], CborError> {
        let end = self.i.checked_add(n).ok_or(CborError)?;
        let s = self.b.get(self.i..end).ok_or(CborError)?;
        self.i = end;
        Ok(s)
    }

    fn head(&mut self) -> Result<(u8, u64), CborError> {
        let ib = self.take(1)?[0];
        let (major, info) = (ib >> 5, ib & 0x1F);
        let v = match info {
            0..=23 => info as u64,
            24 => self.take(1)?[0] as u64,
            25 => u16::from_be_bytes(self.take(2)?.try_into().map_err(|_| CborError)?) as u64,
            26 => u32::from_be_bytes(self.take(4)?.try_into().map_err(|_| CborError)?) as u64,
            27 => u64::from_be_bytes(self.take(8)?.try_into().map_err(|_| CborError)?),
            // 28-30 rezervovano, 31 = neurcita delka: CTAP2 je nepouziva.
            _ => return Err(CborError),
        };
        Ok((major, v))
    }

    fn value(&mut self, depth: usize) -> Result<Value, CborError> {
        if depth > MAX_DEPTH {
            return Err(CborError);
        }
        let (major, v) = self.head()?;
        Ok(match major {
            0 => Value::Uint(v),
            1 => Value::Nint(v),
            2 => Value::Bytes(self.take(usize::try_from(v).map_err(|_| CborError)?)?.to_vec()),
            3 => {
                let s = self.take(usize::try_from(v).map_err(|_| CborError)?)?;
                Value::Text(String::from(core::str::from_utf8(s).map_err(|_| CborError)?))
            }
            4 => {
                if v > MAX_ITEMS {
                    return Err(CborError);
                }
                let mut a = Vec::new();
                for _ in 0..v {
                    a.push(self.value(depth + 1)?);
                }
                Value::Array(a)
            }
            5 => {
                if v > MAX_ITEMS {
                    return Err(CborError);
                }
                let mut m: Vec<(Value, Value)> = Vec::new();
                for _ in 0..v {
                    let k = self.value(depth + 1)?;
                    let val = self.value(depth + 1)?;
                    if m.iter().any(|(x, _)| *x == k) {
                        return Err(CborError);
                    }
                    m.push((k, val));
                }
                Value::Map(m)
            }
            7 => match v {
                20 => Value::Bool(false),
                21 => Value::Bool(true),
                22 => Value::Null,
                _ => return Err(CborError),
            },
            _ => return Err(CborError),
        })
    }
}

/// Mapa s celociselnymi klici (pozadavky CTAP2).
pub fn int_map(items: Vec<(i64, Value)>) -> Value {
    Value::Map(items.into_iter().map(|(k, v)| (Value::int(k), v)).collect())
}

/// Mapa s textovymi klici (entity, volby, rozsireni).
pub fn text_map(items: Vec<(&str, Value)>) -> Value {
    Value::Map(items.into_iter().map(|(k, v)| (Value::text(k), v)).collect())
}
