//! Simulovany bezpecnostni klic CTAP2.1 (jen testy): GetInfo, clientPIN
//! (protokoly 1 a 2, 8 pokusu), registrace ES256 / EdDSA, prihlaseni,
//! `hmac-secret`, prenos CTAPHID ([`SoftHid`]). Pravidla podle CTAP 2.1;
//! neni to bezpecny klic (tajemstvi v pameti procesu, deterministicky
//! generator [`SeedRng`]).

use alloc::collections::VecDeque;
use alloc::vec;
use alloc::vec::Vec;

use hmac::{Hmac, Mac};
use p256::ecdsa::signature::Signer;
use rand_core::{CryptoRng, RngCore};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

use super::cbor::{self, Value, int_map, text_map};
use super::cose::{self, PublicKey};
use super::hid::{self, Assembler, BROADCAST, PACKET, cmd};
use super::hidclient::HidDevice;
use super::msg::{AuthData, command, flag, permission, pin_cmd, status};
use super::pin::{self, Protocol};

/// Deterministicky generator pro testy: SHA-256(seme || citac).
pub struct SeedRng {
    seed: [u8; 32],
    counter: u64,
    buf: [u8; 32],
    used: usize,
}

impl SeedRng {
    pub fn new(seed: u64) -> Self {
        let mut s = [0u8; 32];
        s[..8].copy_from_slice(&seed.to_le_bytes());
        SeedRng { seed: s, counter: 0, buf: [0; 32], used: 32 }
    }
}

impl RngCore for SeedRng {
    fn next_u32(&mut self) -> u32 {
        let mut b = [0u8; 4];
        self.fill_bytes(&mut b);
        u32::from_le_bytes(b)
    }

    fn next_u64(&mut self) -> u64 {
        let mut b = [0u8; 8];
        self.fill_bytes(&mut b);
        u64::from_le_bytes(b)
    }

    fn fill_bytes(&mut self, dest: &mut [u8]) {
        for d in dest {
            if self.used == 32 {
                let mut h = Sha256::new();
                h.update(self.seed);
                h.update(self.counter.to_le_bytes());
                self.buf = h.finalize().into();
                self.counter += 1;
                self.used = 0;
            }
            *d = self.buf[self.used];
            self.used += 1;
        }
    }

    fn try_fill_bytes(&mut self, dest: &mut [u8]) -> Result<(), rand_core::Error> {
        self.fill_bytes(dest);
        Ok(())
    }
}

impl CryptoRng for SeedRng {}

enum CredKey {
    Es256(p256::ecdsa::SigningKey),
    Ed25519(ed25519_dalek::SigningKey),
}

struct Cred {
    id: Vec<u8>,
    rp_hash: [u8; 32],
    key: CredKey,
    /// Tajemstvi hmac-secret s UV a bez UV (CTAP 2.1, 12.5).
    random_uv: [u8; 32],
    random_no_uv: [u8; 32],
}

impl Cred {
    fn public(&self) -> PublicKey {
        match &self.key {
            CredKey::Es256(k) => {
                let (x, y) = cose::xy(&p256::PublicKey::from(k.verifying_key()));
                PublicKey::Es256 { x, y }
            }
            CredKey::Ed25519(k) => PublicKey::Ed25519(k.verifying_key().to_bytes()),
        }
    }

    fn sign(&self, msg: &[u8]) -> Vec<u8> {
        match &self.key {
            CredKey::Es256(k) => {
                let s: p256::ecdsa::Signature = k.sign(msg);
                s.to_der().as_bytes().to_vec()
            }
            CredKey::Ed25519(k) => {
                use ed25519_dalek::Signer as _;
                k.sign(msg).to_bytes().to_vec()
            }
        }
    }
}

/// Simulovany klic.
pub struct SoftKey {
    rng: SeedRng,
    pub aaguid: [u8; 16],
    /// Uzivatel se klice dotkne (jinak `OPERATION_DENIED`).
    pub touch: bool,
    /// Pocet doteku, ktere si klic vyzadal.
    pub touches: u32,
    pin_hash: Option<[u8; 16]>,
    pub pin_retries: u8,
    ka: p256::SecretKey,
    token: [u8; 32],
    token_perms: u64,
    creds: Vec<Cred>,
    pub sign_count: u32,
    /// Podporovane protokoly PIN/UV (vychozi [2, 1]).
    pub protocols: Vec<u64>,
    /// Klic umi jen CTAP 2.0 (GetPinToken bez opravneni).
    pub ctap20_only: bool,
    /// Registrovat EdDSA misto ES256 (kdyz ho platforma nabidne).
    pub prefer_ed25519: bool,
    /// Pocitadlo podpisu se nezvysuje (klon klice).
    pub frozen_counter: bool,
}

type Resp = Result<Value, u8>;

impl SoftKey {
    pub fn new(seed: u64) -> Self {
        let mut rng = SeedRng::new(seed);
        let ka = p256::SecretKey::random(&mut rng);
        let mut token = [0u8; 32];
        rng.fill_bytes(&mut token);
        let mut aaguid = [0u8; 16];
        rng.fill_bytes(&mut aaguid);
        SoftKey {
            rng,
            aaguid,
            touch: true,
            touches: 0,
            pin_hash: None,
            pin_retries: 8,
            ka,
            token,
            token_perms: 0,
            creds: Vec::new(),
            sign_count: 0,
            protocols: vec![2, 1],
            ctap20_only: false,
            prefer_ed25519: false,
            frozen_counter: false,
        }
    }

    /// Klic s nastavenym PINem.
    pub fn with_pin(seed: u64, pin_value: &str) -> Self {
        let mut k = Self::new(seed);
        k.pin_hash = Some(pin::pin_hash(pin_value.as_bytes()));
        k
    }

    pub fn credentials(&self) -> usize {
        self.creds.len()
    }

    /// Zpracuje zpravu `prikaz || CBOR`, vrati `stav || CBOR`.
    pub fn handle(&mut self, req: &[u8]) -> Vec<u8> {
        let Some((&c, body)) = req.split_first() else {
            return vec![status::INVALID_LENGTH];
        };
        let params = if body.is_empty() {
            Value::Map(Vec::new())
        } else {
            match cbor::decode(body) {
                Ok(v) => v,
                Err(_) => return vec![status::INVALID_CBOR],
            }
        };
        let r = match c {
            command::GET_INFO => Ok(self.info()),
            command::CLIENT_PIN => self.client_pin(&params),
            command::MAKE_CREDENTIAL => self.make_credential(&params),
            command::GET_ASSERTION => self.get_assertion(&params),
            _ => Err(status::INVALID_COMMAND),
        };
        match r {
            Ok(v) => {
                let mut out = vec![status::OK];
                out.extend_from_slice(&v.encode());
                out
            }
            Err(s) => vec![s],
        }
    }

    fn info(&self) -> Value {
        let mut versions = vec![Value::text("FIDO_2_0")];
        if !self.ctap20_only {
            versions.push(Value::text("FIDO_2_1"));
        }
        let mut options = vec![("clientPin", Value::Bool(self.pin_hash.is_some())), ("rk", Value::Bool(false)), ("up", Value::Bool(true))];
        if !self.ctap20_only {
            options.push(("pinUvAuthToken", Value::Bool(true)));
        }
        int_map(vec![
            (1, Value::Array(versions)),
            (2, Value::Array(vec![Value::text("hmac-secret")])),
            (3, Value::bytes(&self.aaguid)),
            (4, text_map(options)),
            (6, Value::Array(self.protocols.iter().map(|p| Value::Uint(*p)).collect())),
        ])
    }

    fn proto(&self, v: Option<&Value>) -> Result<Protocol, u8> {
        let n = v.and_then(Value::as_int).ok_or(status::MISSING_PARAMETER)? as u64;
        if !self.protocols.contains(&n) {
            return Err(status::INVALID_PARAMETER);
        }
        Protocol::from_u64(n).ok_or(status::INVALID_PARAMETER)
    }

    fn regenerate(&mut self) {
        self.ka = p256::SecretKey::random(&mut self.rng);
    }

    fn client_pin(&mut self, p: &Value) -> Resp {
        let proto = self.proto(p.get_int(1))?;
        let sub = p.get_int(2).and_then(Value::as_int).ok_or(status::MISSING_PARAMETER)? as u64;
        let shared = |me: &Self| -> Result<pin::Shared, u8> {
            let peer = p.get_int(3).ok_or(status::MISSING_PARAMETER)?;
            pin::decapsulate(&me.ka, proto, peer).ok_or(status::INVALID_PARAMETER)
        };
        match sub {
            pin_cmd::GET_RETRIES => Ok(int_map(vec![(3, Value::Uint(self.pin_retries as u64))])),
            pin_cmd::GET_KEY_AGREEMENT => {
                let (x, y) = cose::xy(&self.ka.public_key());
                Ok(int_map(vec![(1, cose::ecdh_key(&x, &y))]))
            }
            pin_cmd::SET_PIN => {
                if self.pin_hash.is_some() {
                    return Err(status::NOT_ALLOWED);
                }
                let s = shared(self)?;
                let enc = p.get_int(5).and_then(Value::as_bytes).ok_or(status::MISSING_PARAMETER)?;
                let auth = p.get_int(4).and_then(Value::as_bytes).ok_or(status::MISSING_PARAMETER)?;
                if !s.verify(enc, auth) {
                    return Err(status::PIN_AUTH_INVALID);
                }
                let padded = s.decrypt(enc).ok_or(status::PIN_POLICY_VIOLATION)?;
                if padded.len() != 64 {
                    return Err(status::PIN_POLICY_VIOLATION);
                }
                let len = padded.iter().position(|b| *b == 0).unwrap_or(64);
                if len < 4 || len > 63 {
                    return Err(status::PIN_POLICY_VIOLATION);
                }
                self.pin_hash = Some(pin::pin_hash(&padded[..len]));
                self.pin_retries = 8;
                Ok(Value::Map(Vec::new()))
            }
            pin_cmd::GET_PIN_TOKEN | pin_cmd::GET_TOKEN_WITH_PERMISSIONS => {
                if sub == pin_cmd::GET_TOKEN_WITH_PERMISSIONS && self.ctap20_only {
                    return Err(status::INVALID_PARAMETER);
                }
                let Some(want) = self.pin_hash else {
                    return Err(status::PIN_NOT_SET);
                };
                if self.pin_retries == 0 {
                    return Err(status::PIN_BLOCKED);
                }
                let s = shared(self)?;
                let enc = p.get_int(6).and_then(Value::as_bytes).ok_or(status::MISSING_PARAMETER)?;
                self.pin_retries -= 1;
                let got = s.decrypt(enc).ok_or(status::INVALID_PARAMETER)?;
                if got.len() != 16 || !bool::from(got.as_slice().ct_eq(&want)) {
                    self.regenerate();
                    return Err(if self.pin_retries == 0 { status::PIN_BLOCKED } else { status::PIN_INVALID });
                }
                self.pin_retries = 8;
                self.rng.fill_bytes(&mut self.token);
                self.token_perms = if sub == pin_cmd::GET_PIN_TOKEN {
                    permission::MAKE_CREDENTIAL | permission::GET_ASSERTION
                } else {
                    p.get_int(9).and_then(Value::as_int).ok_or(status::MISSING_PARAMETER)? as u64
                };
                let token = self.token;
                let enc = s.encrypt(&mut self.rng, &token).ok_or(status::INVALID_PARAMETER)?;
                Ok(int_map(vec![(2, Value::Bytes(enc))]))
            }
            _ => Err(status::INVALID_PARAMETER),
        }
    }

    /// Overi `pinUvAuthParam`; `Ok(true)` = UV.
    fn check_uv(&self, p: &Value, param_key: i64, proto_key: i64, cdh: &[u8], perm: u64) -> Result<bool, u8> {
        match p.get_int(param_key).and_then(Value::as_bytes) {
            Some(param) => {
                let proto = self.proto(p.get_int(proto_key))?;
                if self.pin_hash.is_none() {
                    return Err(status::PIN_NOT_SET);
                }
                if !pin::verify(proto, &self.token, cdh, param) || self.token_perms & perm == 0 {
                    return Err(status::PIN_AUTH_INVALID);
                }
                Ok(true)
            }
            None => Ok(false),
        }
    }

    fn user_presence(&mut self) -> Result<(), u8> {
        self.touches += 1;
        if self.touch { Ok(()) } else { Err(status::OPERATION_DENIED) }
    }

    fn next_count(&mut self) -> u32 {
        if !self.frozen_counter {
            self.sign_count += 1;
        }
        self.sign_count
    }

    fn make_credential(&mut self, p: &Value) -> Resp {
        let cdh = p.get_int(1).and_then(Value::as_bytes).ok_or(status::MISSING_PARAMETER)?.to_vec();
        let rp_id = p.get_int(2).and_then(|r| r.get_text("id")).and_then(Value::as_text).ok_or(status::MISSING_PARAMETER)?;
        let rp_hash: [u8; 32] = Sha256::digest(rp_id.as_bytes()).into();
        let algs: Vec<i64> = p
            .get_int(4)
            .and_then(Value::as_array)
            .ok_or(status::MISSING_PARAMETER)?
            .iter()
            .filter_map(|a| a.get_text("alg").and_then(Value::as_int))
            .collect();
        let uv = self.check_uv(p, 8, 9, &cdh, permission::MAKE_CREDENTIAL)?;
        if self.pin_hash.is_some() && !uv {
            return Err(status::PUAT_REQUIRED);
        }
        if let Some(ex) = p.get_int(5).and_then(Value::as_array) {
            let hit = ex
                .iter()
                .filter_map(|d| d.get_text("id").and_then(Value::as_bytes))
                .any(|id| self.creds.iter().any(|c| c.id == id && c.rp_hash == rp_hash));
            if hit {
                self.user_presence()?;
                return Err(status::CREDENTIAL_EXCLUDED);
            }
        }
        let use_ed = self.prefer_ed25519 && algs.contains(&cose::ALG_EDDSA);
        if !use_ed && !algs.contains(&cose::ALG_ES256) && !algs.contains(&cose::ALG_EDDSA) {
            return Err(status::UNSUPPORTED_ALGORITHM);
        }
        let hmac_ext = p.get_int(6).and_then(|e| e.get_text("hmac-secret")).and_then(Value::as_bool) == Some(true);
        self.user_presence()?;
        let key = if use_ed || !algs.contains(&cose::ALG_ES256) {
            let mut b = [0u8; 32];
            self.rng.fill_bytes(&mut b);
            CredKey::Ed25519(ed25519_dalek::SigningKey::from_bytes(&b))
        } else {
            CredKey::Es256(p256::ecdsa::SigningKey::random(&mut self.rng))
        };
        let mut id = vec![0u8; 32];
        self.rng.fill_bytes(&mut id);
        let mut random_uv = [0u8; 32];
        let mut random_no_uv = [0u8; 32];
        self.rng.fill_bytes(&mut random_uv);
        self.rng.fill_bytes(&mut random_no_uv);
        let cred = Cred { id: id.clone(), rp_hash, key, random_uv, random_no_uv };
        let mut flags = flag::UP | flag::AT;
        if uv {
            flags |= flag::UV;
        }
        let extensions = hmac_ext.then(|| text_map(vec![("hmac-secret", Value::Bool(true))]));
        if extensions.is_some() {
            flags |= flag::ED;
        }
        let ad = AuthData {
            rp_id_hash: rp_hash,
            flags,
            sign_count: self.next_count(),
            credential: Some((self.aaguid, id, cred.public().to_cose())),
            extensions,
        };
        self.creds.push(cred);
        Ok(int_map(vec![(1, Value::text("none")), (2, Value::Bytes(ad.encode())), (3, Value::Map(Vec::new()))]))
    }

    fn get_assertion(&mut self, p: &Value) -> Resp {
        let rp_id = p.get_int(1).and_then(Value::as_text).ok_or(status::MISSING_PARAMETER)?;
        let rp_hash: [u8; 32] = Sha256::digest(rp_id.as_bytes()).into();
        let cdh = p.get_int(2).and_then(Value::as_bytes).ok_or(status::MISSING_PARAMETER)?.to_vec();
        let allow: Vec<Vec<u8>> = p
            .get_int(3)
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(|d| d.get_text("id").and_then(Value::as_bytes)).map(<[u8]>::to_vec).collect())
            .unwrap_or_default();
        let uv = self.check_uv(p, 6, 7, &cdh, permission::GET_ASSERTION)?;
        let Some(idx) = self.creds.iter().position(|c| c.rp_hash == rp_hash && allow.contains(&c.id)) else {
            return Err(status::NO_CREDENTIALS);
        };
        // hmac-secret: sul zasifrovana sdilenym tajemstvim.
        let mut hmac_out = None;
        if let Some(hs) = p.get_int(4).and_then(|e| e.get_text("hmac-secret")) {
            let proto = match hs.get_int(4) {
                Some(v) => self.proto(Some(v))?,
                None => Protocol::V1,
            };
            let peer = hs.get_int(1).ok_or(status::MISSING_PARAMETER)?;
            let s = pin::decapsulate(&self.ka, proto, peer).ok_or(status::INVALID_PARAMETER)?;
            let salt_enc = hs.get_int(2).and_then(Value::as_bytes).ok_or(status::MISSING_PARAMETER)?;
            let salt_auth = hs.get_int(3).and_then(Value::as_bytes).ok_or(status::MISSING_PARAMETER)?;
            if !s.verify(salt_enc, salt_auth) {
                return Err(status::INVALID_PARAMETER);
            }
            let salt = s.decrypt(salt_enc).ok_or(status::INVALID_PARAMETER)?;
            if salt.len() != 32 && salt.len() != 64 {
                return Err(status::INVALID_LENGTH);
            }
            let c = &self.creds[idx];
            let secret = if uv { c.random_uv } else { c.random_no_uv };
            let mut out = Vec::new();
            for part in salt.chunks(32) {
                let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(&secret).expect("klic HMAC");
                mac.update(part);
                out.extend_from_slice(&mac.finalize().into_bytes());
            }
            hmac_out = Some(s.encrypt(&mut self.rng, &out).ok_or(status::INVALID_PARAMETER)?);
        }
        self.user_presence()?;
        let mut flags = flag::UP;
        if uv {
            flags |= flag::UV;
        }
        let extensions = hmac_out.map(|o| text_map(vec![("hmac-secret", Value::Bytes(o))]));
        if extensions.is_some() {
            flags |= flag::ED;
        }
        let ad = AuthData { rp_id_hash: rp_hash, flags, sign_count: self.next_count(), credential: None, extensions }.encode();
        let mut signed = ad.clone();
        signed.extend_from_slice(&cdh);
        let c = &self.creds[idx];
        let sig = c.sign(&signed);
        Ok(int_map(vec![
            (1, text_map(vec![("id", Value::bytes(&c.id)), ("type", Value::text("public-key"))])),
            (2, Value::Bytes(ad)),
            (3, Value::Bytes(sig)),
        ]))
    }
}

impl super::client::Transport for SoftKey {
    fn transact(&mut self, request: &[u8]) -> super::client::CtapResult<Vec<u8>> {
        Ok(self.handle(request))
    }
}


/// Simulovany klic za prenosem CTAPHID (pakety po 64 B).
pub struct SoftHid {
    pub key: SoftKey,
    asm: Assembler,
    next_cid: u32,
    out: VecDeque<[u8; PACKET]>,
    /// Pred odpovedi CBOR poslat KEEPALIVE "ceka na dotek".
    pub keepalive: bool,
}

impl SoftHid {
    pub fn new(key: SoftKey) -> Self {
        SoftHid { key, asm: Assembler::default(), next_cid: 0x0100_0001, out: VecDeque::new(), keepalive: true }
    }

    fn send(&mut self, cid: u32, command: u8, data: &[u8]) {
        if let Ok(ps) = hid::fragment(cid, command, data) {
            self.out.extend(ps);
        }
    }
}

impl HidDevice for SoftHid {
    fn write(&mut self, p: &[u8; PACKET]) -> abi::Result<()> {
        match self.asm.push(p) {
            Ok(Some(m)) => match m.command {
                cmd::INIT if m.data.len() == 8 => {
                    let cid = if m.cid == BROADCAST {
                        self.next_cid += 1;
                        self.next_cid
                    } else {
                        m.cid
                    };
                    let nonce: [u8; 8] = m.data[..8].try_into().map_err(|_| abi::Error::InvalidArgs)?;
                    let resp = hid::init_response(&nonce, cid, hid::caps::CBOR | hid::caps::NMSG | hid::caps::WINK);
                    self.send(m.cid, cmd::INIT, &resp);
                }
                cmd::CBOR if m.cid != BROADCAST => {
                    if self.keepalive {
                        self.send(m.cid, cmd::KEEPALIVE, &[hid::keepalive::UP_NEEDED]);
                    }
                    let r = self.key.handle(&m.data);
                    self.send(m.cid, cmd::CBOR, &r);
                }
                cmd::PING => self.send(m.cid, cmd::PING, &m.data),
                _ => self.send(m.cid, cmd::ERROR, &[hid::err::INVALID_CMD]),
            },
            Ok(None) => {}
            Err(_) => {
                let cid = u32::from_be_bytes([p[0], p[1], p[2], p[3]]);
                self.send(cid, cmd::ERROR, &[hid::err::INVALID_SEQ]);
            }
        }
        Ok(())
    }

    fn read(&mut self, _timeout_ms: u64) -> abi::Result<[u8; PACKET]> {
        self.out.pop_front().ok_or(abi::Error::TimedOut)
    }
}
