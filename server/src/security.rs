//! Passwords, tokens, codes and rate limits. Only established primitives:
//! Argon2id (the `argon2` crate) for passwords, the operating system's
//! random generator (`getrandom`) for tokens and codes, SHA-256 for storing
//! tokens and codes, constant-time comparison (`subtle`) for secrets.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Instant;

use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::{Algorithm, Argon2, Params, Version};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

/// Argon2id cost. `PRODUCTION` follows OWASP's recommendation (19 MiB, two
/// passes, one lane); tests use a cheap setting so the suite stays fast.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cost {
    pub memory_kib: u32,
    pub passes: u32,
    pub lanes: u32,
}

impl Cost {
    pub const PRODUCTION: Self = Self {
        memory_kib: 19_456,
        passes: 2,
        lanes: 1,
    };
    pub const TEST: Self = Self {
        memory_kib: 1024,
        passes: 1,
        lanes: 1,
    };

    fn hasher(self) -> Argon2<'static> {
        let params = Params::new(self.memory_kib, self.passes, self.lanes, None)
            .expect("valid Argon2 parameters");
        Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
    }
}

/// `n` random bytes from the operating system.
pub fn random_bytes<const N: usize>() -> [u8; N] {
    let mut b = [0u8; N];
    getrandom::fill(&mut b).expect("the operating system's random generator works");
    b
}

/// A 256-bit random token, URL-safe base64.
pub fn token() -> String {
    URL_SAFE_NO_PAD.encode(random_bytes::<32>())
}

/// A random identifier (128 bits, hex).
pub fn id() -> String {
    random_bytes::<16>()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// An 8-digit code, uniformly distributed.
pub fn code() -> String {
    loop {
        let n = u32::from_le_bytes(random_bytes::<4>());
        // Reject the top values so every code is equally likely.
        if n < u32::MAX - (u32::MAX % 100_000_000) {
            return format!("{:08}", n % 100_000_000);
        }
    }
}

/// How tokens and codes are stored: their SHA-256, hex.
pub fn digest(secret: &str) -> String {
    Sha256::digest(secret.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Equal in constant time (for digests and proofs of the same length).
pub fn same(a: &str, b: &str) -> bool {
    a.len() == b.len() && bool::from(a.as_bytes().ct_eq(b.as_bytes()))
}

pub fn hash_password(password: &str, cost: Cost) -> String {
    let salt = SaltString::encode_b64(&random_bytes::<16>()).expect("16 bytes is a valid salt");
    cost.hasher()
        .hash_password(password.as_bytes(), &salt)
        .expect("hashing with valid parameters works")
        .to_string()
}

/// Whether `password` matches the stored hash (its own parameters apply).
pub fn verify_password(password: &str, hash: &str) -> bool {
    match PasswordHash::new(hash) {
        Ok(parsed) => Argon2::default()
            .verify_password(password.as_bytes(), &parsed)
            .is_ok(),
        Err(_) => false,
    }
}

/// Why a password is refused (NIST SP 800-63B: length, no composition
/// rules).
pub fn password_problem(password: &str) -> Option<&'static str> {
    let n = password.chars().count();
    if n < 12 {
        return Some("the password must have at least 12 characters");
    }
    if n > 256 {
        return Some("the password must have at most 256 characters");
    }
    let mut chars = password.chars();
    let first = chars.next();
    if chars.all(|c| Some(c) == first) {
        return Some("the password must not repeat one character");
    }
    None
}

/// A normalized email address, or `None` if it is not one.
pub fn email(input: &str) -> Option<String> {
    let e = input.trim().to_lowercase();
    let (local, domain) = e.split_once('@')?;
    let ok = !local.is_empty()
        && domain.contains('.')
        && !domain.starts_with('.')
        && !domain.ends_with('.')
        && e.len() <= 254
        && !e.chars().any(|c| c.is_whitespace() || c.is_control())
        && !domain.contains('@');
    ok.then_some(e)
}

/// The progressive lockout after `failures` wrong passwords: none for the
/// first four, then 30 s doubling per failure, at most 15 minutes.
pub fn lockout_ms(failures: i64) -> i64 {
    if failures < 5 {
        return 0;
    }
    let shift = u32::try_from((failures - 5).min(10)).unwrap_or(10);
    (30_000i64 << shift).min(15 * 60_000)
}

/// Token buckets per key (an IP address and an endpoint class, or an email).
#[derive(Default)]
pub struct Limits {
    buckets: Mutex<HashMap<String, (f64, Instant)>>,
}

impl Limits {
    /// Takes one token from `key`'s bucket of `capacity` tokens refilling at
    /// `per_minute`; `false` when empty.
    pub fn allow(&self, key: &str, capacity: f64, per_minute: f64) -> bool {
        let now = Instant::now();
        let mut buckets = self.buckets.lock().unwrap_or_else(|e| e.into_inner());
        if buckets.len() > 100_000 {
            // Forget full buckets; an attacker cannot grow memory forever.
            buckets.retain(|_, (tokens, at)| {
                *tokens + at.elapsed().as_secs_f64() * per_minute / 60.0 < capacity
            });
        }
        let (tokens, at) = buckets.entry(key.to_owned()).or_insert((capacity, now));
        *tokens =
            (*tokens + now.duration_since(*at).as_secs_f64() * per_minute / 60.0).min(capacity);
        *at = now;
        if *tokens >= 1.0 {
            *tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passwords_hash_with_argon2id_and_verify() {
        let h = hash_password("correct horse battery", Cost::TEST);
        assert!(h.starts_with("$argon2id$v=19$"), "{h}");
        assert!(verify_password("correct horse battery", &h));
        assert!(!verify_password("correct horse batterY", &h));
        assert!(!verify_password("x", "not a hash"));
        assert_ne!(
            h,
            hash_password("correct horse battery", Cost::TEST),
            "salted"
        );
        let p = hash_password("correct horse battery", Cost::PRODUCTION);
        assert!(p.contains("m=19456,t=2,p=1"), "{p}");
    }

    #[test]
    fn tokens_codes_and_ids_are_random_and_well_formed() {
        let (a, b) = (token(), token());
        assert_ne!(a, b);
        assert_eq!(a.len(), 43);
        let c = code();
        assert_eq!(c.len(), 8);
        assert!(c.bytes().all(|b| b.is_ascii_digit()));
        assert_eq!(id().len(), 32);
        assert_eq!(digest("x").len(), 64);
        assert!(same(&digest("x"), &digest("x")));
        assert!(!same(&digest("x"), &digest("y")));
        assert!(!same("ab", "abc"));
    }

    #[test]
    fn password_and_email_rules() {
        assert!(password_problem("short").is_some());
        assert!(password_problem("aaaaaaaaaaaaaaa").is_some());
        assert!(password_problem("a reasonably long passphrase").is_none());
        assert_eq!(
            email("  Me@Example.ORG ").as_deref(),
            Some("me@example.org")
        );
        for bad in ["", "x", "x@y", "@y.z", "x@.z", "a b@c.d", "x@y.z."] {
            assert_eq!(email(bad), None, "{bad}");
        }
    }

    #[test]
    fn lockout_grows_and_is_capped() {
        assert_eq!(lockout_ms(4), 0);
        assert_eq!(lockout_ms(5), 30_000);
        assert_eq!(lockout_ms(6), 60_000);
        assert_eq!(lockout_ms(50), 15 * 60_000);
    }

    #[test]
    fn buckets_limit_bursts() {
        let l = Limits::default();
        assert!((0..3).all(|_| l.allow("k", 3.0, 1.0)));
        assert!(!l.allow("k", 3.0, 1.0));
        assert!(l.allow("other", 3.0, 1.0), "keys are independent");
    }
}
