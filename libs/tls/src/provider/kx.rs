//! Vymena klicu: X25519, secp256r1, secp384r1 (ECDHE).

use alloc::boxed::Box;
use alloc::vec::Vec;

use rustls::crypto::{ActiveKeyExchange, SharedSecret, SupportedKxGroup};
use rustls::{Error, NamedGroup, PeerMisbehaved};

use super::{Platform, Rng};

/// Generator nad nahodou `platform`. Generatory klicu RustCrypto chybu
/// nahody neumi vratit (RngCore `fill_bytes` by panikaril): dostupnost se
/// overi predem a vrati se chyba.
fn rng(platform: &'static Platform) -> Result<Rng, Error> {
    let mut probe = [0u8; 1];
    if !(platform.random)(&mut probe) {
        return Err(Error::FailedToGetRandomBytes);
    }
    Ok(Rng(platform))
}

fn bad_share() -> Error {
    Error::from(PeerMisbehaved::InvalidKeyShare)
}

/// Skupina nese platformu (kazdy `Client` muze mit vlastni).
#[derive(Debug)]
struct X25519(&'static Platform);

struct X25519Exchange {
    secret: x25519_dalek::EphemeralSecret,
    public: x25519_dalek::PublicKey,
}

impl SupportedKxGroup for X25519 {
    fn start(&self) -> Result<Box<dyn ActiveKeyExchange>, Error> {
        let secret = x25519_dalek::EphemeralSecret::random_from_rng(rng(self.0)?);
        let public = x25519_dalek::PublicKey::from(&secret);
        Ok(Box::new(X25519Exchange { secret, public }))
    }

    fn name(&self) -> NamedGroup {
        NamedGroup::X25519
    }
}

impl ActiveKeyExchange for X25519Exchange {
    fn complete(self: Box<Self>, peer: &[u8]) -> Result<SharedSecret, Error> {
        let peer: [u8; 32] = peer.try_into().map_err(|_| bad_share())?;
        let shared = self.secret.diffie_hellman(&peer.into());
        // Nulove sdilene tajemstvi = verejny klic male podgrupy (RFC 7748 6.1).
        if !shared.was_contributory() {
            return Err(bad_share());
        }
        Ok(SharedSecret::from(&shared.as_bytes()[..]))
    }

    fn pub_key(&self) -> &[u8] {
        self.public.as_bytes()
    }

    fn group(&self) -> NamedGroup {
        NamedGroup::X25519
    }
}

macro_rules! nist_kx {
    ($group:ident, $exchange:ident, $curve:ident, $named:expr) => {
        #[derive(Debug)]
        struct $group(&'static Platform);

        struct $exchange {
            secret: $curve::ecdh::EphemeralSecret,
            public: Vec<u8>,
        }

        impl SupportedKxGroup for $group {
            fn start(&self) -> Result<Box<dyn ActiveKeyExchange>, Error> {
                let mut r = rng(self.0)?;
                let secret = $curve::ecdh::EphemeralSecret::random(&mut r);
                let public = $curve::EncodedPoint::from(secret.public_key()).as_bytes().to_vec();
                Ok(Box::new($exchange { secret, public }))
            }

            fn name(&self) -> NamedGroup {
                $named
            }
        }

        impl ActiveKeyExchange for $exchange {
            fn complete(self: Box<Self>, peer: &[u8]) -> Result<SharedSecret, Error> {
                // Jen nekomprimovany bod (RFC 8446 4.2.8.2); from_sec1_bytes
                // overi, ze lezi na krivce.
                if peer.first() != Some(&4) {
                    return Err(bad_share());
                }
                let peer = $curve::PublicKey::from_sec1_bytes(peer).map_err(|_| bad_share())?;
                let shared = self.secret.diffie_hellman(&peer);
                Ok(SharedSecret::from(&shared.raw_secret_bytes()[..]))
            }

            fn pub_key(&self) -> &[u8] {
                &self.public
            }

            fn group(&self) -> NamedGroup {
                $named
            }
        }
    };
}

nist_kx!(Secp256r1, Secp256r1Exchange, p256, NamedGroup::secp256r1);
nist_kx!(Secp384r1, Secp384r1Exchange, p384, NamedGroup::secp384r1);

/// Skupiny v poradi preference pro `platform`. Kazda skupina je jedna
/// mala alokace na cely beh programu (rustls chce `&'static`); `provider`
/// se vola jednou pro kazdy `Client`.
pub fn all(platform: &'static Platform) -> alloc::vec::Vec<&'static dyn SupportedKxGroup> {
    alloc::vec![
        Box::leak(Box::new(X25519(platform))) as &'static dyn SupportedKxGroup,
        Box::leak(Box::new(Secp256r1(platform))),
        Box::leak(Box::new(Secp384r1(platform))),
    ]
}
