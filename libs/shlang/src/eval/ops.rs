//! Operatory: aritmetika, porovnani, rovnost.
//!
//! Zadne skryte prevody: velikost se nescita s cislem (`size > 100` je
//! chyba s radou `100B`), text s cislem take ne. Jedina vyjimka: cele
//! cislo se v aritmetice s desetinnym prevede na desetinne.

use alloc::string::String;
use alloc::vec::Vec;
use core::cmp::Ordering;

use crate::ast::{BinOp, Lit, Num, UnOp, Unit};
use crate::error::{ShError, code};
use crate::value::Value;

fn type_err(op: &str, a: &Value, b: &Value) -> ShError {
    let mut e = ShError::new(
        code::TYPE,
        alloc::format!("operaci {op} nejde použít na {} a {}", a.type_name(), b.type_name()),
    );
    let unit_hint = |v: &Value| matches!(v, Value::Size(_) | Value::Duration(_));
    if (unit_hint(a) && matches!(b, Value::Int(_) | Value::Float(_)))
        || (unit_hint(b) && matches!(a, Value::Int(_) | Value::Float(_)))
    {
        e = e.hint("k číslu napište jednotku, např. 100MB, 1KiB, 30s, 1h");
    }
    e
}

fn overflow() -> ShError {
    ShError::new(code::ARITH, "přetečení čísla")
}

/// Hodnota literalu.
pub fn lit_value(l: &Lit) -> Result<Value, ShError> {
    Ok(match l {
        Lit::Null => Value::Null,
        Lit::Bool(b) => Value::Bool(*b),
        Lit::Int(i) => Value::Int(*i),
        Lit::Float(f) => Value::Float(*f),
        Lit::Str(s) => Value::Str(s.clone()),
        Lit::Unit { value, unit } => unit_value(*value, *unit)?,
    })
}

pub fn unit_value(value: Num, unit: Unit) -> Result<Value, ShError> {
    let f = unit.factor();
    let n: i64 = match value {
        Num::Int(i) => {
            let v = (i as i128) * (f as i128);
            i64::try_from(v).map_err(|_| overflow())?
        }
        Num::Float(x) => {
            let v = x * f as f64;
            if !v.is_finite() || v.abs() > 9.2e18 {
                return Err(overflow());
            }
            round(v)
        }
    };
    Ok(if unit.is_size() { Value::Size(n) } else { Value::Duration(n) })
}

/// Zaokrouhleni bez `std` (f64::round neni v `core`).
fn round(v: f64) -> i64 {
    if v >= 0.0 { (v + 0.5) as i64 } else { (v - 0.5) as i64 }
}

fn as_f64(v: &Value) -> Option<f64> {
    match v {
        Value::Int(i) => Some(*i as f64),
        Value::Float(f) => Some(*f),
        _ => None,
    }
}

pub fn unary(op: UnOp, v: Value) -> Result<Value, ShError> {
    match (op, v) {
        (UnOp::Not, Value::Bool(b)) => Ok(Value::Bool(!b)),
        (UnOp::Neg, Value::Int(i)) => i.checked_neg().map(Value::Int).ok_or_else(overflow),
        (UnOp::Neg, Value::Float(f)) => Ok(Value::Float(-f)),
        (UnOp::Neg, Value::Size(s)) => s.checked_neg().map(Value::Size).ok_or_else(overflow),
        (UnOp::Neg, Value::Duration(d)) => d.checked_neg().map(Value::Duration).ok_or_else(overflow),
        (UnOp::Not, v) => Err(ShError::new(code::TYPE, alloc::format!("'!' patří jen na bool, ne na {}", v.type_name()))),
        (UnOp::Neg, v) => Err(ShError::new(code::TYPE, alloc::format!("'-' nejde použít na {}", v.type_name()))),
    }
}

/// Aritmetika a porovnani (bez `&&` a `||`, ty resi interpret).
pub fn binary(op: BinOp, a: Value, b: Value) -> Result<Value, ShError> {
    use Value::*;
    let sym = op.symbol();
    match op {
        BinOp::Eq => return equal(&a, &b).map(Bool),
        BinOp::Ne => return equal(&a, &b).map(|e| Bool(!e)),
        BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
            let o = compare(&a, &b)?;
            return Ok(Bool(match op {
                BinOp::Lt => o == Ordering::Less,
                BinOp::Le => o != Ordering::Greater,
                BinOp::Gt => o == Ordering::Greater,
                _ => o != Ordering::Less,
            }));
        }
        _ => {}
    }
    let r = match (op, &a, &b) {
        (BinOp::Add, Int(x), Int(y)) => x.checked_add(*y).map(Int).ok_or_else(overflow)?,
        (BinOp::Sub, Int(x), Int(y)) => x.checked_sub(*y).map(Int).ok_or_else(overflow)?,
        (BinOp::Mul, Int(x), Int(y)) => x.checked_mul(*y).map(Int).ok_or_else(overflow)?,
        (BinOp::Div, Int(x), Int(y)) => {
            if *y == 0 {
                return Err(ShError::new(code::ARITH, "dělení nulou"));
            }
            x.checked_div(*y).map(Int).ok_or_else(overflow)?
        }
        (BinOp::Rem, Int(x), Int(y)) => {
            if *y == 0 {
                return Err(ShError::new(code::ARITH, "dělení nulou"));
            }
            x.checked_rem(*y).map(Int).ok_or_else(overflow)?
        }
        (BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div, Int(_) | Float(_), Int(_) | Float(_)) => {
            let (x, y) = (as_f64(&a).unwrap(), as_f64(&b).unwrap());
            Float(match op {
                BinOp::Add => x + y,
                BinOp::Sub => x - y,
                BinOp::Mul => x * y,
                _ => {
                    if y == 0.0 {
                        return Err(ShError::new(code::ARITH, "dělení nulou"));
                    }
                    x / y
                }
            })
        }
        // velikosti
        (BinOp::Add, Size(x), Size(y)) => x.checked_add(*y).map(Size).ok_or_else(overflow)?,
        (BinOp::Sub, Size(x), Size(y)) => x.checked_sub(*y).map(Size).ok_or_else(overflow)?,
        (BinOp::Mul, Size(x), Int(y)) | (BinOp::Mul, Int(y), Size(x)) => x.checked_mul(*y).map(Size).ok_or_else(overflow)?,
        (BinOp::Mul, Size(x), Float(y)) | (BinOp::Mul, Float(y), Size(x)) => Size(round(*x as f64 * y)),
        (BinOp::Div, Size(x), Int(y)) => {
            if *y == 0 {
                return Err(ShError::new(code::ARITH, "dělení nulou"));
            }
            x.checked_div(*y).map(Size).ok_or_else(overflow)?
        }
        (BinOp::Div, Size(x), Size(y)) => {
            if *y == 0 {
                return Err(ShError::new(code::ARITH, "dělení nulou"));
            }
            Float(*x as f64 / *y as f64)
        }
        // doby a casy
        (BinOp::Add, Duration(x), Duration(y)) => x.checked_add(*y).map(Duration).ok_or_else(overflow)?,
        (BinOp::Sub, Duration(x), Duration(y)) => x.checked_sub(*y).map(Duration).ok_or_else(overflow)?,
        (BinOp::Mul, Duration(x), Int(y)) | (BinOp::Mul, Int(y), Duration(x)) => {
            x.checked_mul(*y).map(Duration).ok_or_else(overflow)?
        }
        (BinOp::Div, Duration(x), Int(y)) => {
            if *y == 0 {
                return Err(ShError::new(code::ARITH, "dělení nulou"));
            }
            x.checked_div(*y).map(Duration).ok_or_else(overflow)?
        }
        (BinOp::Div, Duration(x), Duration(y)) => {
            if *y == 0 {
                return Err(ShError::new(code::ARITH, "dělení nulou"));
            }
            Float(*x as f64 / *y as f64)
        }
        (BinOp::Add, Time(t), Duration(d)) | (BinOp::Add, Duration(d), Time(t)) => {
            t.checked_add(*d).map(Time).ok_or_else(overflow)?
        }
        (BinOp::Sub, Time(t), Duration(d)) => t.checked_sub(*d).map(Time).ok_or_else(overflow)?,
        (BinOp::Sub, Time(x), Time(y)) => x.checked_sub(*y).map(Duration).ok_or_else(overflow)?,
        // text, seznamy, zaznamy
        (BinOp::Add, Str(x), Str(y)) => {
            let mut s = String::from(x.as_str());
            s.push_str(y);
            Str(s)
        }
        (BinOp::Add, List(x), List(y)) => {
            let mut v: Vec<Value> = x.clone();
            v.extend(y.iter().cloned());
            List(v)
        }
        (BinOp::Add, Record(x), Record(y)) => {
            let mut r = x.clone();
            for (k, v) in y {
                match r.iter_mut().find(|(rk, _)| rk == k) {
                    Some(slot) => slot.1 = v.clone(),
                    None => r.push((k.clone(), v.clone())),
                }
            }
            Record(r)
        }
        _ => return Err(type_err(sym, &a, &b)),
    };
    Ok(r)
}

/// Rovnost. Ruzne typy (krome cisel a `null`) jsou chyba - `size == 5`
/// by jinak tise vratilo `false`.
pub fn equal(a: &Value, b: &Value) -> Result<bool, ShError> {
    use Value::*;
    Ok(match (a, b) {
        (Null, Null) => true,
        (Null, _) | (_, Null) => false,
        (Int(x), Float(y)) | (Float(y), Int(x)) => (*x as f64) == *y,
        (List(x), List(y)) => {
            if x.len() != y.len() {
                return Ok(false);
            }
            for (p, q) in x.iter().zip(y) {
                if !equal(p, q)? {
                    return Ok(false);
                }
            }
            true
        }
        _ if core::mem::discriminant(a) == core::mem::discriminant(b) => a == b,
        _ => return Err(type_err("==", a, b)),
    })
}

/// Porovnani pro `<`, `>` a `sort`.
pub fn compare(a: &Value, b: &Value) -> Result<Ordering, ShError> {
    use Value::*;
    let o = match (a, b) {
        (Int(x), Int(y)) | (Size(x), Size(y)) | (Duration(x), Duration(y)) | (Time(x), Time(y)) => x.cmp(y),
        (Int(_) | Float(_), Int(_) | Float(_)) => {
            let (x, y) = (as_f64(a).unwrap(), as_f64(b).unwrap());
            x.partial_cmp(&y).unwrap_or(Ordering::Equal)
        }
        (Str(x), Str(y)) => x.cmp(y),
        (Bool(x), Bool(y)) => x.cmp(y),
        (Null, Null) => Ordering::Equal,
        _ => return Err(type_err("porovnání", a, b)),
    };
    Ok(o)
}
