//! Strana platformy CTAP2: GetInfo, clientPIN (protokoly 1 a 2),
//! registrace s `hmac-secret` a prihlaseni s overenim podpisu.
//!
//! Prenos je za traitem [`Transport`] (accountd: CTAPHID pres usb-hid,
//! testy: simulovany klic primo).

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use rand_core::CryptoRngCore;
use zeroize::Zeroize;

use super::cbor::{self, Value, int_map, text_map};
use super::cose::{self, PublicKey};
use super::msg::{AuthData, Info, command, flag, permission, pin_cmd, rp_id_hash, status};
use super::pin::{self, Protocol, Shared};

/// Chyba CTAP2.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CtapError {
    /// Stav z klice ([`status`]).
    Status(u8),
    /// Chyba prenosu (klic odpojen, casovy limit).
    Transport(abi::Error),
    /// Odpoved nedava smysl (spatne CBOR, chybi pole, spatny podpis).
    Invalid,
    /// Klic nepodporuje, co je potreba (protokol PIN, hmac-secret).
    Unsupported,
}

pub type CtapResult<T> = core::result::Result<T, CtapError>;

/// Prenos jedne zpravy CTAP2: `prikaz || CBOR` -> `stav || CBOR`.
pub trait Transport {
    fn transact(&mut self, request: &[u8]) -> CtapResult<Vec<u8>>;
}

impl<T: Transport + ?Sized> Transport for &mut T {
    fn transact(&mut self, request: &[u8]) -> CtapResult<Vec<u8>> {
        (**self).transact(request)
    }
}

/// Identita spolehajici strany pro prihlaseni do systemu.
pub const RP_ID: &str = "magnumopus.local";

/// Token PIN/UV (vynuluje se pri zahozeni).
pub struct Token {
    pub proto: Protocol,
    token: Vec<u8>,
}

impl Drop for Token {
    fn drop(&mut self) {
        self.token.zeroize();
    }
}

impl Token {
    /// `pinUvAuthParam` nad `clientDataHash`.
    pub fn param(&self, cdh: &[u8; 32]) -> Vec<u8> {
        pin::authenticate(self.proto, &self.token, cdh)
    }
}

/// Nove registrovany klic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Credential {
    pub id: Vec<u8>,
    pub public: PublicKey,
    pub sign_count: u32,
    pub flags: u8,
    pub hmac_secret: bool,
}

/// Overene prihlaseni.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Assertion {
    pub id: Vec<u8>,
    pub flags: u8,
    pub sign_count: u32,
    /// Vystup `hmac-secret` (32 B), kdyz platforma poslala sul.
    pub hmac_output: Option<[u8; 32]>,
}

pub struct Client<T> {
    pub transport: T,
    pub info: Info,
}

fn request(cmd: u8, params: Option<Value>) -> Vec<u8> {
    let mut v = vec![cmd];
    if let Some(p) = params {
        v.extend_from_slice(&p.encode());
    }
    v
}

impl<T: Transport> Client<T> {
    /// Pripoji se ke klici (`authenticatorGetInfo`).
    pub fn new(mut transport: T) -> CtapResult<Self> {
        let resp = call(&mut transport, request(command::GET_INFO, None))?;
        let info = Info::parse(&resp).ok_or(CtapError::Invalid)?;
        if !info.versions.iter().any(|v| v.starts_with("FIDO_2")) {
            return Err(CtapError::Unsupported);
        }
        Ok(Client { transport, info })
    }

    fn call(&mut self, req: Vec<u8>) -> CtapResult<Value> {
        call(&mut self.transport, req)
    }

    /// Nejlepsi spolecny protokol PIN/UV.
    pub fn protocol(&self) -> CtapResult<Protocol> {
        if self.info.pin_protocols.contains(&2) {
            Ok(Protocol::V2)
        } else if self.info.pin_protocols.contains(&1) || self.info.pin_protocols.is_empty() && self.info.client_pin.is_some() {
            Ok(Protocol::V1)
        } else {
            Err(CtapError::Unsupported)
        }
    }

    /// Ma klic nastaveny PIN?
    pub fn pin_set(&self) -> bool {
        self.info.client_pin == Some(true)
    }

    fn client_pin(&mut self, proto: Protocol, sub: u64, extra: Vec<(i64, Value)>) -> CtapResult<Value> {
        let mut m = vec![(1, Value::Uint(proto.number())), (2, Value::Uint(sub))];
        m.extend(extra);
        self.call(request(command::CLIENT_PIN, Some(int_map(m))))
    }

    /// Dohoda o sdilenem tajemstvi s klicem.
    fn agree(&mut self, rng: &mut impl CryptoRngCore) -> CtapResult<(Value, Shared)> {
        let proto = self.protocol()?;
        let r = self.client_pin(proto, pin_cmd::GET_KEY_AGREEMENT, vec![])?;
        let peer = r.get_int(1).ok_or(CtapError::Invalid)?;
        pin::encapsulate(rng, proto, peer).ok_or(CtapError::Invalid)
    }

    /// Zbyvajici pokusy PINu klice.
    pub fn pin_retries(&mut self) -> CtapResult<u64> {
        let proto = self.protocol()?;
        let r = self.client_pin(proto, pin_cmd::GET_RETRIES, vec![])?;
        r.get_int(3).and_then(Value::as_int).map(|v| v as u64).ok_or(CtapError::Invalid)
    }

    /// Nastavi PIN klice, ktery PIN jeste nema.
    pub fn set_pin(&mut self, rng: &mut impl CryptoRngCore, new_pin: &[u8]) -> CtapResult<()> {
        let (platform, shared) = self.agree(rng)?;
        let mut padded = pin::pad_pin(new_pin).ok_or(CtapError::Status(status::PIN_POLICY_VIOLATION))?;
        let enc = shared.encrypt(rng, &padded).ok_or(CtapError::Invalid);
        padded.zeroize();
        let enc = enc?;
        let auth = shared.authenticate(&enc);
        self.client_pin(shared.proto, pin_cmd::SET_PIN, vec![(3, platform), (4, Value::Bytes(auth)), (5, Value::Bytes(enc))])?;
        self.info.client_pin = Some(true);
        Ok(())
    }

    /// Token PIN/UV z PINu klice s opravnenim pro prihlaseni a registraci.
    pub fn pin_token(&mut self, rng: &mut impl CryptoRngCore, pin_value: &[u8]) -> CtapResult<Token> {
        let (platform, shared) = self.agree(rng)?;
        let mut hash = pin::pin_hash(pin_value);
        let enc = shared.encrypt(rng, &hash).ok_or(CtapError::Invalid);
        hash.zeroize();
        let enc = enc?;
        let r = if self.info.pin_uv_token {
            self.client_pin(
                shared.proto,
                pin_cmd::GET_TOKEN_WITH_PERMISSIONS,
                vec![
                    (3, platform),
                    (6, Value::Bytes(enc)),
                    (9, Value::Uint(permission::MAKE_CREDENTIAL | permission::GET_ASSERTION)),
                    (10, Value::text(RP_ID)),
                ],
            )?
        } else {
            self.client_pin(shared.proto, pin_cmd::GET_PIN_TOKEN, vec![(3, platform), (6, Value::Bytes(enc))])?
        };
        let enc_token = r.get_int(2).and_then(Value::as_bytes).ok_or(CtapError::Invalid)?;
        let token = shared.decrypt(enc_token).ok_or(CtapError::Invalid)?;
        if token.is_empty() {
            return Err(CtapError::Invalid);
        }
        Ok(Token { proto: shared.proto, token })
    }

    /// Registrace (`authenticatorMakeCredential`).
    pub fn make_credential(
        &mut self,
        cdh: &[u8; 32],
        user_id: &[u8],
        user_name: &str,
        exclude: &[Vec<u8>],
        token: Option<&Token>,
    ) -> CtapResult<Credential> {
        let want_hmac = self.info.has_extension("hmac-secret");
        let mut m = vec![
            (1, Value::bytes(cdh)),
            (2, text_map(vec![("id", Value::text(RP_ID)), ("name", Value::text("MagnumOpus"))])),
            (3, text_map(vec![("id", Value::bytes(user_id)), ("name", Value::text(user_name))])),
            (
                4,
                Value::Array(vec![
                    text_map(vec![("alg", Value::int(cose::ALG_ES256)), ("type", Value::text("public-key"))]),
                    text_map(vec![("alg", Value::int(cose::ALG_EDDSA)), ("type", Value::text("public-key"))]),
                ]),
            ),
        ];
        if !exclude.is_empty() {
            m.push((5, Value::Array(exclude.iter().map(|id| descriptor(id)).collect())));
        }
        if want_hmac {
            m.push((6, text_map(vec![("hmac-secret", Value::Bool(true))])));
        }
        m.push((7, text_map(vec![("rk", Value::Bool(false))])));
        if let Some(t) = token {
            m.push((8, Value::Bytes(t.param(cdh))));
            m.push((9, Value::Uint(t.proto.number())));
        }
        let r = self.call(request(command::MAKE_CREDENTIAL, Some(int_map(m))))?;
        let ad = r.get_int(2).and_then(Value::as_bytes).and_then(AuthData::parse).ok_or(CtapError::Invalid)?;
        if ad.rp_id_hash != rp_id_hash(RP_ID) || ad.flags & flag::UP == 0 {
            return Err(CtapError::Invalid);
        }
        let (_, id, key) = ad.credential.as_ref().ok_or(CtapError::Invalid)?;
        let public = PublicKey::from_cose(key).ok_or(CtapError::Status(status::UNSUPPORTED_ALGORITHM))?;
        let hmac_secret = ad
            .extensions
            .as_ref()
            .and_then(|e| e.get_text("hmac-secret"))
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if token.is_some() && ad.flags & flag::UV == 0 {
            return Err(CtapError::Invalid);
        }
        Ok(Credential { id: id.clone(), public, sign_count: ad.sign_count, flags: ad.flags, hmac_secret })
    }

    /// Prihlaseni (`authenticatorGetAssertion`) a overeni podpisu proti
    /// ulozenym klicum `allow` (id, verejny klic). `salt` = sul pro
    /// `hmac-secret`.
    pub fn get_assertion(
        &mut self,
        rng: &mut impl CryptoRngCore,
        cdh: &[u8; 32],
        allow: &[(Vec<u8>, PublicKey)],
        token: Option<&Token>,
        salt: Option<&[u8; 32]>,
    ) -> CtapResult<Assertion> {
        let mut m = vec![
            (1, Value::text(RP_ID)),
            (2, Value::bytes(cdh)),
            (3, Value::Array(allow.iter().map(|(id, _)| descriptor(id)).collect())),
        ];
        let mut shared = None;
        if let Some(salt) = salt {
            let (platform, s) = self.agree(rng)?;
            let enc = s.encrypt(rng, salt).ok_or(CtapError::Invalid)?;
            let auth = s.authenticate(&enc);
            let mut hm = vec![(1, platform), (2, Value::Bytes(enc)), (3, Value::Bytes(auth))];
            if s.proto == Protocol::V2 {
                hm.push((4, Value::Uint(2)));
            }
            m.push((4, text_map(vec![("hmac-secret", int_map(hm))])));
            shared = Some(s);
        }
        if let Some(t) = token {
            m.push((6, Value::Bytes(t.param(cdh))));
            m.push((7, Value::Uint(t.proto.number())));
        }
        let r = self.call(request(command::GET_ASSERTION, Some(int_map(m))))?;
        let raw_ad = r.get_int(2).and_then(Value::as_bytes).ok_or(CtapError::Invalid)?;
        let sig = r.get_int(3).and_then(Value::as_bytes).ok_or(CtapError::Invalid)?;
        // Pri jedinem klici v allowList smi klic credential vynechat.
        let id = match r.get_int(1) {
            Some(c) => c.get_text("id").and_then(Value::as_bytes).ok_or(CtapError::Invalid)?.to_vec(),
            None if allow.len() == 1 => allow[0].0.clone(),
            None => return Err(CtapError::Invalid),
        };
        let (_, public) = allow.iter().find(|(a, _)| *a == id).ok_or(CtapError::Invalid)?;
        let ad = AuthData::parse(raw_ad).ok_or(CtapError::Invalid)?;
        if ad.rp_id_hash != rp_id_hash(RP_ID) || ad.flags & flag::UP == 0 {
            return Err(CtapError::Invalid);
        }
        if token.is_some() && ad.flags & flag::UV == 0 {
            return Err(CtapError::Invalid);
        }
        let mut signed = raw_ad.to_vec();
        signed.extend_from_slice(cdh);
        if !public.verify(&signed, sig) {
            return Err(CtapError::Invalid);
        }
        let hmac_output = match (shared, &ad.extensions) {
            (Some(s), Some(ext)) => {
                let enc = ext.get_text("hmac-secret").and_then(Value::as_bytes).ok_or(CtapError::Invalid)?;
                let mut out = s.decrypt(enc).ok_or(CtapError::Invalid)?;
                let r: Option<[u8; 32]> = out.get(..32).and_then(|b| b.try_into().ok());
                out.zeroize();
                Some(r.ok_or(CtapError::Invalid)?)
            }
            (Some(_), None) => return Err(CtapError::Unsupported),
            _ => None,
        };
        Ok(Assertion { id, flags: ad.flags, sign_count: ad.sign_count, hmac_output })
    }
}

fn descriptor(id: &[u8]) -> Value {
    text_map(vec![("id", Value::bytes(id)), ("type", Value::text("public-key"))])
}

fn call(t: &mut impl Transport, req: Vec<u8>) -> CtapResult<Value> {
    let resp = t.transact(&req)?;
    let (&st, body) = resp.split_first().ok_or(CtapError::Invalid)?;
    if st != status::OK {
        return Err(CtapError::Status(st));
    }
    if body.is_empty() {
        return Ok(Value::Map(Vec::new()));
    }
    cbor::decode(body).map_err(|_| CtapError::Invalid)
}

/// Lidsky popis stavu klice (pro log a odpoved; bez tajemstvi).
pub fn describe(e: CtapError) -> String {
    use alloc::format;
    match e {
        CtapError::Status(status::PIN_INVALID) => String::from("spatny PIN klice"),
        CtapError::Status(status::PIN_BLOCKED) => String::from("PIN klice zablokovany"),
        CtapError::Status(status::PIN_AUTH_BLOCKED) => String::from("klic je treba odpojit a pripojit"),
        CtapError::Status(status::PIN_NOT_SET) => String::from("klic nema PIN"),
        CtapError::Status(status::NO_CREDENTIALS) => String::from("klic neni registrovany k uctu"),
        CtapError::Status(status::OPERATION_DENIED) | CtapError::Status(status::USER_ACTION_TIMEOUT) => {
            String::from("klic nebyl potvrzen dotekem")
        }
        CtapError::Status(s) => format!("klic vratil chybu 0x{s:02x}"),
        CtapError::Transport(e) => format!("prenos: {e}"),
        CtapError::Invalid => String::from("neplatna odpoved klice"),
        CtapError::Unsupported => String::from("klic nepodporuje potrebne funkce"),
    }
}
