//! SHA-256/384 a HMAC pro rustls.

use alloc::boxed::Box;

use hmac::Mac;
use rustls::crypto::{hash, hmac as rhmac};
use sha2::{Digest, Sha256, Sha384};

macro_rules! hash_impl {
    ($name:ident, $ctx:ident, $ty:ty, $alg:expr, $len:expr) => {
        struct $name;

        impl hash::Hash for $name {
            fn start(&self) -> Box<dyn hash::Context> {
                Box::new($ctx(<$ty>::new()))
            }

            fn hash(&self, data: &[u8]) -> hash::Output {
                hash::Output::new(&<$ty>::digest(data)[..])
            }

            fn output_len(&self) -> usize {
                $len
            }

            fn algorithm(&self) -> hash::HashAlgorithm {
                $alg
            }
        }

        struct $ctx($ty);

        impl hash::Context for $ctx {
            fn fork_finish(&self) -> hash::Output {
                hash::Output::new(&self.0.clone().finalize()[..])
            }

            fn fork(&self) -> Box<dyn hash::Context> {
                Box::new($ctx(self.0.clone()))
            }

            fn finish(self: Box<Self>) -> hash::Output {
                hash::Output::new(&self.0.finalize()[..])
            }

            fn update(&mut self, data: &[u8]) {
                self.0.update(data);
            }
        }
    };
}

hash_impl!(Sha256Hash, Sha256Ctx, Sha256, hash::HashAlgorithm::SHA256, 32);
hash_impl!(Sha384Hash, Sha384Ctx, Sha384, hash::HashAlgorithm::SHA384, 48);

pub const SHA256: &dyn hash::Hash = &Sha256Hash;
pub const SHA384: &dyn hash::Hash = &Sha384Hash;

macro_rules! hmac_impl {
    ($name:ident, $key:ident, $ty:ty, $len:expr) => {
        struct $name;

        impl rhmac::Hmac for $name {
            fn with_key(&self, key: &[u8]) -> Box<dyn rhmac::Key> {
                // HMAC prijme klic libovolne delky (delsi zahashuje).
                Box::new($key(hmac::Hmac::<$ty>::new_from_slice(key).expect("HMAC: libovolna delka klice")))
            }

            fn hash_output_len(&self) -> usize {
                $len
            }
        }

        struct $key(hmac::Hmac<$ty>);

        impl rhmac::Key for $key {
            fn sign_concat(&self, first: &[u8], middle: &[&[u8]], last: &[u8]) -> rhmac::Tag {
                let mut m = self.0.clone();
                m.update(first);
                for part in middle {
                    m.update(part);
                }
                m.update(last);
                rhmac::Tag::new(&m.finalize().into_bytes()[..])
            }

            fn tag_len(&self) -> usize {
                $len
            }
        }
    };
}

hmac_impl!(HmacSha256, HmacSha256Key, Sha256, 32);
hmac_impl!(HmacSha384, HmacSha384Key, Sha384, 48);

pub const HMAC_SHA256: &dyn rhmac::Hmac = &HmacSha256;
pub const HMAC_SHA384: &dyn rhmac::Hmac = &HmacSha384;
