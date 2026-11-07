//! Sifrovani zaznamu TLS 1.3 (AES-GCM, ChaCha20-Poly1305).
//!
//! Zaznam TLS 1.3 (RFC 8446 5.2): data + typ obsahu (1 B), zasifrovano
//! AEAD s nonce = IV xor cislo zaznamu a s AAD = hlavicka zaznamu.

use alloc::boxed::Box;

use aead::{AeadInPlace, Buffer, KeyInit};
use aes_gcm::{Aes128Gcm, Aes256Gcm};
use chacha20poly1305::ChaCha20Poly1305;
use rustls::crypto::cipher::{
    self, AeadKey, BorrowedPayload, InboundOpaqueMessage, InboundPlainMessage, MessageDecrypter,
    MessageEncrypter, OutboundOpaqueMessage, OutboundPlainMessage, PrefixedPayload, Tls13AeadAlgorithm,
    UnsupportedOperationError,
};
use rustls::{ConnectionTrafficSecrets, ContentType, ProtocolVersion};

/// Delka autentizacniho tagu vsech tri AEAD.
const TAG: usize = 16;

struct OutBuf<'a>(&'a mut PrefixedPayload);

impl AsRef<[u8]> for OutBuf<'_> {
    fn as_ref(&self) -> &[u8] {
        self.0.as_ref()
    }
}

impl AsMut<[u8]> for OutBuf<'_> {
    fn as_mut(&mut self) -> &mut [u8] {
        self.0.as_mut()
    }
}

impl Buffer for OutBuf<'_> {
    fn extend_from_slice(&mut self, other: &[u8]) -> aead::Result<()> {
        self.0.extend_from_slice(other);
        Ok(())
    }

    fn truncate(&mut self, len: usize) {
        self.0.truncate(len)
    }
}

struct InBuf<'a, 'p>(&'a mut BorrowedPayload<'p>);

impl AsRef<[u8]> for InBuf<'_, '_> {
    fn as_ref(&self) -> &[u8] {
        self.0
    }
}

impl AsMut<[u8]> for InBuf<'_, '_> {
    fn as_mut(&mut self) -> &mut [u8] {
        self.0
    }
}

impl Buffer for InBuf<'_, '_> {
    fn extend_from_slice(&mut self, _: &[u8]) -> aead::Result<()> {
        // Desifrovani na miste buffer jen zkracuje (odebere tag).
        Err(aead::Error)
    }

    fn truncate(&mut self, len: usize) {
        self.0.truncate(len)
    }
}

/// Druh AEAD a jeho tajemstvi pro `extract_keys`.
#[derive(Clone, Copy)]
enum Kind {
    Aes128,
    Aes256,
    ChaCha,
}

pub struct Alg(Kind);

pub const AES_128_GCM: Alg = Alg(Kind::Aes128);
pub const AES_256_GCM: Alg = Alg(Kind::Aes256);
pub const CHACHA20_POLY1305: Alg = Alg(Kind::ChaCha);

// Sifra lezi v Box<dyn MessageEncrypter> (jedna alokace na smer), ruzna
// velikost variant nevadi.
#[allow(clippy::large_enum_variant)]
enum Cipher {
    Aes128(Aes128Gcm),
    Aes256(Aes256Gcm),
    ChaCha(ChaCha20Poly1305),
}

impl Cipher {
    fn new(kind: Kind, key: &[u8]) -> Self {
        // Delku klice urcuje `key_len`, rustls preda presne tolik.
        match kind {
            Kind::Aes128 => Self::Aes128(Aes128Gcm::new_from_slice(key).expect("delka klice AES-128")),
            Kind::Aes256 => Self::Aes256(Aes256Gcm::new_from_slice(key).expect("delka klice AES-256")),
            Kind::ChaCha => Self::ChaCha(ChaCha20Poly1305::new_from_slice(key).expect("delka klice ChaCha20")),
        }
    }

    fn encrypt(&self, nonce: &[u8; 12], aad: &[u8], buf: &mut dyn Buffer) -> aead::Result<()> {
        let n = nonce.into();
        match self {
            Self::Aes128(c) => c.encrypt_in_place(n, aad, buf),
            Self::Aes256(c) => c.encrypt_in_place(n, aad, buf),
            Self::ChaCha(c) => c.encrypt_in_place(n, aad, buf),
        }
    }

    fn decrypt(&self, nonce: &[u8; 12], aad: &[u8], buf: &mut dyn Buffer) -> aead::Result<()> {
        let n = nonce.into();
        match self {
            Self::Aes128(c) => c.decrypt_in_place(n, aad, buf),
            Self::Aes256(c) => c.decrypt_in_place(n, aad, buf),
            Self::ChaCha(c) => c.decrypt_in_place(n, aad, buf),
        }
    }
}

impl Tls13AeadAlgorithm for Alg {
    fn encrypter(&self, key: AeadKey, iv: cipher::Iv) -> Box<dyn MessageEncrypter> {
        Box::new(Record(Cipher::new(self.0, key.as_ref()), iv))
    }

    fn decrypter(&self, key: AeadKey, iv: cipher::Iv) -> Box<dyn MessageDecrypter> {
        Box::new(Record(Cipher::new(self.0, key.as_ref()), iv))
    }

    fn key_len(&self) -> usize {
        match self.0 {
            Kind::Aes128 => 16,
            Kind::Aes256 | Kind::ChaCha => 32,
        }
    }

    fn extract_keys(&self, key: AeadKey, iv: cipher::Iv) -> Result<ConnectionTrafficSecrets, UnsupportedOperationError> {
        Ok(match self.0 {
            Kind::Aes128 => ConnectionTrafficSecrets::Aes128Gcm { key, iv },
            Kind::Aes256 => ConnectionTrafficSecrets::Aes256Gcm { key, iv },
            Kind::ChaCha => ConnectionTrafficSecrets::Chacha20Poly1305 { key, iv },
        })
    }
}

struct Record(Cipher, cipher::Iv);

impl MessageEncrypter for Record {
    fn encrypt(&mut self, m: OutboundPlainMessage<'_>, seq: u64) -> Result<OutboundOpaqueMessage, rustls::Error> {
        let total = self.encrypted_payload_len(m.payload.len());
        let mut payload = PrefixedPayload::with_capacity(total);
        let nonce = cipher::Nonce::new(&self.1, seq).0;
        let aad = cipher::make_tls13_aad(total);
        payload.extend_from_chunks(&m.payload);
        payload.extend_from_slice(&m.typ.to_array());
        self.0
            .encrypt(&nonce, &aad, &mut OutBuf(&mut payload))
            .map_err(|_| rustls::Error::EncryptError)?;
        Ok(OutboundOpaqueMessage::new(ContentType::ApplicationData, ProtocolVersion::TLSv1_2, payload))
    }

    fn encrypted_payload_len(&self, payload_len: usize) -> usize {
        payload_len + 1 + TAG
    }
}

impl MessageDecrypter for Record {
    fn decrypt<'a>(&mut self, mut m: InboundOpaqueMessage<'a>, seq: u64) -> Result<InboundPlainMessage<'a>, rustls::Error> {
        let payload = &mut m.payload;
        let nonce = cipher::Nonce::new(&self.1, seq).0;
        let aad = cipher::make_tls13_aad(payload.len());
        self.0
            .decrypt(&nonce, &aad, &mut InBuf(payload))
            .map_err(|_| rustls::Error::DecryptError)?;
        m.into_tls13_unpadded_message()
    }
}
