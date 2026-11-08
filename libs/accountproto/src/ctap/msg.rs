//! Prikazy CTAP2 (CTAP 2.1, kap. 6): kody, stavy a rozbor `authData`.

use alloc::vec::Vec;

use sha2::{Digest, Sha256};

use super::cbor::{self, Value};

/// Prikazy (prvni bajt zpravy CBOR).
pub mod command {
    pub const MAKE_CREDENTIAL: u8 = 0x01;
    pub const GET_ASSERTION: u8 = 0x02;
    pub const GET_INFO: u8 = 0x04;
    pub const CLIENT_PIN: u8 = 0x06;
    pub const RESET: u8 = 0x07;
    pub const GET_NEXT_ASSERTION: u8 = 0x08;
}

/// Podprikazy `authenticatorClientPIN`.
pub mod pin_cmd {
    pub const GET_RETRIES: u64 = 0x01;
    pub const GET_KEY_AGREEMENT: u64 = 0x02;
    pub const SET_PIN: u64 = 0x03;
    pub const CHANGE_PIN: u64 = 0x04;
    pub const GET_PIN_TOKEN: u64 = 0x05;
    pub const GET_TOKEN_WITH_PERMISSIONS: u64 = 0x09;
}

/// Opravneni tokenu PIN/UV (CTAP 2.1).
pub mod permission {
    pub const MAKE_CREDENTIAL: u64 = 0x01;
    pub const GET_ASSERTION: u64 = 0x02;
}

/// Stavove kody (prvni bajt odpovedi).
pub mod status {
    pub const OK: u8 = 0x00;
    pub const INVALID_COMMAND: u8 = 0x01;
    pub const INVALID_PARAMETER: u8 = 0x02;
    pub const INVALID_LENGTH: u8 = 0x03;
    pub const INVALID_CBOR: u8 = 0x12;
    pub const MISSING_PARAMETER: u8 = 0x14;
    pub const CREDENTIAL_EXCLUDED: u8 = 0x19;
    pub const UNSUPPORTED_ALGORITHM: u8 = 0x26;
    pub const OPERATION_DENIED: u8 = 0x27;
    pub const UNSUPPORTED_OPTION: u8 = 0x2B;
    pub const KEEPALIVE_CANCEL: u8 = 0x2D;
    pub const NO_CREDENTIALS: u8 = 0x2E;
    pub const USER_ACTION_TIMEOUT: u8 = 0x2F;
    pub const NOT_ALLOWED: u8 = 0x30;
    pub const PIN_INVALID: u8 = 0x31;
    pub const PIN_BLOCKED: u8 = 0x32;
    pub const PIN_AUTH_INVALID: u8 = 0x33;
    pub const PIN_AUTH_BLOCKED: u8 = 0x34;
    pub const PIN_NOT_SET: u8 = 0x35;
    pub const PUAT_REQUIRED: u8 = 0x36;
    pub const PIN_POLICY_VIOLATION: u8 = 0x37;
}

/// Priznaky v `authData`.
pub mod flag {
    /// Uzivatel pritomen (dotek).
    pub const UP: u8 = 0x01;
    /// Uzivatel overen (PIN klice nebo biometrie).
    pub const UV: u8 = 0x04;
    /// Obsahuje data noveho prihlasovaciho udaje.
    pub const AT: u8 = 0x40;
    /// Obsahuje rozsireni.
    pub const ED: u8 = 0x80;
}

/// Rozebrana `authData`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthData {
    pub rp_id_hash: [u8; 32],
    pub flags: u8,
    pub sign_count: u32,
    /// Jen u registrace: AAGUID, id prihlasovaciho udaje, klic COSE.
    pub credential: Option<([u8; 16], Vec<u8>, Value)>,
    pub extensions: Option<Value>,
}

impl AuthData {
    pub fn parse(b: &[u8]) -> Option<AuthData> {
        if b.len() < 37 {
            return None;
        }
        let mut rp_id_hash = [0u8; 32];
        rp_id_hash.copy_from_slice(&b[..32]);
        let flags = b[32];
        let sign_count = u32::from_be_bytes(b[33..37].try_into().ok()?);
        let mut i = 37;
        let mut credential = None;
        if flags & flag::AT != 0 {
            let aaguid: [u8; 16] = b.get(i..i + 16)?.try_into().ok()?;
            i += 16;
            let n = u16::from_be_bytes(b.get(i..i + 2)?.try_into().ok()?) as usize;
            i += 2;
            let id = b.get(i..i + n)?.to_vec();
            i += n;
            let (key, used) = cbor::decode_prefix(b.get(i..)?).ok()?;
            i += used;
            credential = Some((aaguid, id, key));
        }
        let mut extensions = None;
        if flags & flag::ED != 0 {
            let (ext, used) = cbor::decode_prefix(b.get(i..)?).ok()?;
            i += used;
            extensions = Some(ext);
        }
        if i != b.len() {
            return None;
        }
        Some(AuthData { rp_id_hash, flags, sign_count, credential, extensions })
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut v = self.rp_id_hash.to_vec();
        v.push(self.flags);
        v.extend_from_slice(&self.sign_count.to_be_bytes());
        if let Some((aaguid, id, key)) = &self.credential {
            v.extend_from_slice(aaguid);
            v.extend_from_slice(&(id.len() as u16).to_be_bytes());
            v.extend_from_slice(id);
            v.extend_from_slice(&key.encode());
        }
        if let Some(ext) = &self.extensions {
            v.extend_from_slice(&ext.encode());
        }
        v
    }
}

/// `SHA-256(rp id)`.
pub fn rp_id_hash(rp_id: &str) -> [u8; 32] {
    Sha256::digest(rp_id.as_bytes()).into()
}

/// Informace o klici (`authenticatorGetInfo`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Info {
    pub versions: Vec<alloc::string::String>,
    pub extensions: Vec<alloc::string::String>,
    pub aaguid: [u8; 16],
    /// Volba `clientPin`: `None` = klic PIN neumi, `Some(false)` = umi,
    /// ale neni nastaveny.
    pub client_pin: Option<bool>,
    /// Klic umi token s opravnenimi (CTAP 2.1, `pinUvAuthToken`).
    pub pin_uv_token: bool,
    pub pin_protocols: Vec<u64>,
}

impl Info {
    pub fn parse(v: &Value) -> Option<Info> {
        let texts = |k: i64| -> Vec<alloc::string::String> {
            v.get_int(k)
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(|x| x.as_text().map(alloc::string::String::from)).collect())
                .unwrap_or_default()
        };
        let options = v.get_int(4);
        let opt = |name: &str| options.and_then(|o| o.get_text(name)).and_then(Value::as_bool);
        Some(Info {
            versions: texts(1),
            extensions: texts(2),
            aaguid: v.get_int(3)?.as_bytes()?.try_into().ok()?,
            client_pin: opt("clientPin"),
            pin_uv_token: opt("pinUvAuthToken").unwrap_or(false),
            pin_protocols: v
                .get_int(6)
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(|x| x.as_int()).map(|x| x as u64).collect())
                .unwrap_or_default(),
        })
    }

    pub fn has_extension(&self, name: &str) -> bool {
        self.extensions.iter().any(|e| e == name)
    }
}
