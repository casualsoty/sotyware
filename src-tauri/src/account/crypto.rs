//! The vault's cryptography (ADR-032), built only from established
//! primitives (RustCrypto): XChaCha20-Poly1305 (AEAD, 192-bit random
//! nonces), HKDF-SHA256, and X25519 sealed boxes (libsodium's
//! `crypto_box_seal` construction, crate `crypto_box`). No primitive is
//! implemented here; this module only fixes formats, key derivations and
//! the associated data that binds each ciphertext to its account and key.
//!
//! Keys:
//! - the **account key** `AK` (256-bit, random) encrypts the vault;
//! - the **recovery key** `RK` (256-bit, random, shown once) wraps `AK`
//!   through `HKDF-SHA256(RK, salt "sotyware recovery v1", info account)`;
//! - each **device key pair** (X25519) receives `AK` in a sealed box;
//! - the **key proof** `HKDF-SHA256(AK, info "sotyware key proof v1" ‖
//!   account)` tells the service that a device holds `AK` without telling it
//!   `AK`.
//!
//! Every envelope names its format and version, and decryption refuses
//! anything else: there is no silent downgrade.

use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use chacha20poly1305::aead::{Aead, AeadCore, KeyInit, OsRng, Payload};
use chacha20poly1305::{Key, XChaCha20Poly1305, XNonce};
use hkdf::Hkdf;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use zeroize::{Zeroize, Zeroizing};

/// Why a cryptographic operation failed. Never carries key material.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CryptoError {
    /// Not a format/version/algorithm this build reads (or malformed).
    Format(String),
    /// Authentication failed: wrong key, or the data was changed.
    Decrypt,
    /// A recovery key with a typo (checksum) or the wrong length.
    RecoveryKey,
}

impl std::fmt::Display for CryptoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Format(m) => write!(f, "unsupported or damaged data: {m}"),
            Self::Decrypt => f.write_str("the data could not be decrypted with this key (wrong key, or the data was changed)"),
            Self::RecoveryKey => f.write_str("this is not a valid recovery key; check it for typos"),
        }
    }
}

/// A 256-bit key, wiped from memory when dropped.
#[derive(Clone, PartialEq, Eq)]
pub struct SecretKey(Zeroizing<[u8; 32]>);

impl std::fmt::Debug for SecretKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SecretKey(…)")
    }
}

impl SecretKey {
    pub fn generate() -> Self {
        // The operating system's generator, through `aead`'s OsRng.
        let key = XChaCha20Poly1305::generate_key(&mut OsRng);
        let mut k = [0u8; 32];
        k.copy_from_slice(&key);
        let secret = Self(Zeroizing::new(k));
        k.zeroize();
        secret
    }

    pub fn from_bytes(b: [u8; 32]) -> Self {
        Self(Zeroizing::new(b))
    }

    pub fn bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Base64, for the OS credential store only.
    pub fn to_base64(&self) -> Zeroizing<String> {
        Zeroizing::new(B64.encode(*self.0))
    }

    pub fn from_base64(s: &str) -> Result<Self, CryptoError> {
        let v = Zeroizing::new(
            B64.decode(s.trim())
                .map_err(|_| CryptoError::Format("key".into()))?,
        );
        let arr: [u8; 32] = v
            .as_slice()
            .try_into()
            .map_err(|_| CryptoError::Format("key length".into()))?;
        Ok(Self::from_bytes(arr))
    }

    /// A short, non-secret identifier of the key (which key encrypted a
    /// vault), derived one-way.
    pub fn id(&self) -> String {
        Sha256::digest([b"sotyware key id v1".as_slice(), self.0.as_slice()].concat())
            .iter()
            .take(8)
            .map(|b| format!("{b:02x}"))
            .collect()
    }
}

fn hkdf(ikm: &[u8], salt: &[u8], info: &[u8]) -> SecretKey {
    let mut okm = [0u8; 32];
    Hkdf::<Sha256>::new(Some(salt), ikm)
        .expand(info, &mut okm)
        .expect("32 bytes is a valid HKDF-SHA256 length");
    let k = SecretKey::from_bytes(okm);
    okm.zeroize();
    k
}

/// The proof sent to the service: one-way from `AK`, bound to the account.
pub fn key_proof(ak: &SecretKey, account: &str) -> String {
    let k = hkdf(
        ak.bytes(),
        b"sotyware key proof salt v1",
        format!("sotyware key proof v1|{account}").as_bytes(),
    );
    B64.encode(k.bytes())
}

// ------------------------------------------------------------ recovery key

/// Crockford's base32 alphabet (no I, L, O, U).
const CROCKFORD: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

fn base32(bytes: &[u8]) -> String {
    let mut out = String::new();
    let (mut buf, mut bits) = (0u32, 0u32);
    for &b in bytes {
        buf = (buf << 8) | u32::from(b);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(char::from(CROCKFORD[((buf >> bits) & 31) as usize]));
        }
    }
    if bits > 0 {
        out.push(char::from(CROCKFORD[((buf << (5 - bits)) & 31) as usize]));
    }
    out
}

fn unbase32(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let (mut buf, mut bits) = (0u32, 0u32);
    for c in s.chars() {
        let c = match c.to_ascii_uppercase() {
            'O' => '0',
            'I' | 'L' => '1',
            c => c,
        };
        let v = CROCKFORD.iter().position(|&a| char::from(a) == c)? as u32;
        buf = (buf << 5) | v;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits) as u8);
            buf &= (1 << bits) - 1;
        }
    }
    Some(out)
}

/// The recovery key as the user writes it down: 52 base32 characters for
/// the 256 bits and 4 for a checksum (20 bits of SHA-256), in groups of 4.
pub fn format_recovery_key(rk: &SecretKey) -> Zeroizing<String> {
    let body = base32(rk.bytes());
    let check = Sha256::digest(rk.bytes());
    let check = base32(&check[..3]);
    let all = Zeroizing::new(format!("{body}{}", &check[..4]));
    Zeroizing::new(
        all.as_bytes()
            .chunks(4)
            .map(|c| std::str::from_utf8(c).unwrap_or_default())
            .collect::<Vec<_>>()
            .join("-"),
    )
}

/// Reads a recovery key typed by the user (any case, spaces or dashes; O
/// read as 0, I and L as 1) and checks its checksum.
pub fn parse_recovery_key(input: &str) -> Result<SecretKey, CryptoError> {
    let clean: Zeroizing<String> = Zeroizing::new(
        input
            .chars()
            .filter(|c| !c.is_whitespace() && *c != '-')
            .collect(),
    );
    if clean.chars().count() != 56 {
        return Err(CryptoError::RecoveryKey);
    }
    let body = Zeroizing::new(unbase32(&clean[..52]).ok_or(CryptoError::RecoveryKey)?);
    let arr: [u8; 32] = body
        .get(..32)
        .and_then(|b| b.try_into().ok())
        .ok_or(CryptoError::RecoveryKey)?;
    let rk = SecretKey::from_bytes(arr);
    let expected = format_recovery_key(&rk);
    let normalized: String = expected.chars().filter(|c| *c != '-').collect();
    let typed: String = clean
        .chars()
        .map(|c| match c.to_ascii_uppercase() {
            'O' => '0',
            'I' | 'L' => '1',
            c => c,
        })
        .collect();
    if normalized != typed {
        return Err(CryptoError::RecoveryKey);
    }
    Ok(rk)
}

// ----------------------------------------------------------------- AEAD

/// One XChaCha20-Poly1305 ciphertext with its format header.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Envelope {
    pub format: String,
    pub v: u32,
    pub alg: String,
    pub kid: String,
    pub nonce: String,
    pub ct: String,
}

const ALG: &str = "xchacha20poly1305";

fn seal_aead(key: &SecretKey, format: &str, kid: &str, aad: &str, plaintext: &[u8]) -> Envelope {
    let cipher = XChaCha20Poly1305::new(Key::from_slice(key.bytes()));
    let nonce = XChaCha20Poly1305::generate_nonce(&mut OsRng);
    let ct = cipher
        .encrypt(
            &nonce,
            Payload {
                msg: plaintext,
                aad: aad.as_bytes(),
            },
        )
        .expect("encryption with a valid key does not fail");
    Envelope {
        format: format.into(),
        v: 1,
        alg: ALG.into(),
        kid: kid.into(),
        nonce: B64.encode(nonce),
        ct: B64.encode(ct),
    }
}

fn open_aead(
    key: &SecretKey,
    env: &Envelope,
    format: &str,
    aad: &str,
) -> Result<Zeroizing<Vec<u8>>, CryptoError> {
    if env.format != format || env.v != 1 || env.alg != ALG {
        return Err(CryptoError::Format(format!(
            "{} v{} {}",
            env.format, env.v, env.alg
        )));
    }
    let nonce = B64
        .decode(&env.nonce)
        .map_err(|_| CryptoError::Format("nonce".into()))?;
    if nonce.len() != 24 {
        return Err(CryptoError::Format("nonce length".into()));
    }
    let ct = B64
        .decode(&env.ct)
        .map_err(|_| CryptoError::Format("ciphertext".into()))?;
    let cipher = XChaCha20Poly1305::new(Key::from_slice(key.bytes()));
    cipher
        .decrypt(
            XNonce::from_slice(&nonce),
            Payload {
                msg: &ct,
                aad: aad.as_bytes(),
            },
        )
        .map(Zeroizing::new)
        .map_err(|_| CryptoError::Decrypt)
}

const VAULT_FORMAT: &str = "sotyware-vault";
const RECOVERY_FORMAT: &str = "sotyware-recovery-wrap";

fn vault_aad(account: &str, kid: &str) -> String {
    format!("{VAULT_FORMAT}|v1|{account}|{kid}")
}

/// Encrypts the vault document for `account` under `AK`.
pub fn encrypt_vault(ak: &SecretKey, account: &str, plaintext: &[u8]) -> String {
    let kid = ak.id();
    let env = seal_aead(ak, VAULT_FORMAT, &kid, &vault_aad(account, &kid), plaintext);
    serde_json::to_string(&env).expect("an envelope serializes")
}

/// Decrypts a vault envelope; refuses another format, version, algorithm,
/// key id or account.
pub fn decrypt_vault(
    ak: &SecretKey,
    account: &str,
    envelope: &str,
) -> Result<Zeroizing<Vec<u8>>, CryptoError> {
    let env: Envelope =
        serde_json::from_str(envelope).map_err(|e| CryptoError::Format(e.to_string()))?;
    if env.kid != ak.id() {
        return Err(CryptoError::Decrypt);
    }
    open_aead(ak, &env, VAULT_FORMAT, &vault_aad(account, &env.kid))
}

/// Wraps `AK` under the recovery key.
pub fn wrap_with_recovery(ak: &SecretKey, rk: &SecretKey, account: &str) -> String {
    let kek = hkdf(rk.bytes(), b"sotyware recovery v1", account.as_bytes());
    let kid = ak.id();
    let env = seal_aead(
        &kek,
        RECOVERY_FORMAT,
        &kid,
        &format!("{RECOVERY_FORMAT}|v1|{account}|{kid}"),
        ak.bytes(),
    );
    serde_json::to_string(&env).expect("an envelope serializes")
}

/// Unwraps `AK` with the recovery key.
pub fn unwrap_with_recovery(
    wrap: &str,
    rk: &SecretKey,
    account: &str,
) -> Result<SecretKey, CryptoError> {
    let env: Envelope =
        serde_json::from_str(wrap).map_err(|e| CryptoError::Format(e.to_string()))?;
    let kek = hkdf(rk.bytes(), b"sotyware recovery v1", account.as_bytes());
    let plain = open_aead(
        &kek,
        &env,
        RECOVERY_FORMAT,
        &format!("{RECOVERY_FORMAT}|v1|{account}|{}", env.kid),
    )?;
    let arr: [u8; 32] = plain
        .as_slice()
        .try_into()
        .map_err(|_| CryptoError::Format("key length".into()))?;
    let ak = SecretKey::from_bytes(arr);
    if ak.id() != env.kid {
        return Err(CryptoError::Decrypt);
    }
    Ok(ak)
}

// ------------------------------------------------------------ device keys

/// A device's X25519 key pair. The secret half stays in the OS credential
/// store; the public half goes to the service and into the vault.
pub struct DeviceKey(crypto_box::SecretKey);

impl DeviceKey {
    pub fn generate() -> Self {
        Self(crypto_box::SecretKey::generate(&mut OsRng))
    }

    pub fn from_secret(k: &SecretKey) -> Self {
        Self(crypto_box::SecretKey::from(*k.bytes()))
    }

    pub fn secret(&self) -> SecretKey {
        SecretKey::from_bytes(self.0.to_bytes())
    }

    pub fn public_base64(&self) -> String {
        B64.encode(self.0.public_key().as_bytes())
    }
}

#[derive(Serialize, Deserialize)]
struct SealedPayload {
    format: String,
    account: String,
    kid: String,
    key: String,
}

/// Seals `AK` to a device's public key (base64), bound to the account.
pub fn seal_for_device(
    ak: &SecretKey,
    account: &str,
    public_key: &str,
) -> Result<String, CryptoError> {
    let pk = B64
        .decode(public_key)
        .map_err(|_| CryptoError::Format("public key".into()))?;
    let pk: [u8; 32] = pk
        .as_slice()
        .try_into()
        .map_err(|_| CryptoError::Format("public key length".into()))?;
    let payload = Zeroizing::new(
        serde_json::to_vec(&SealedPayload {
            format: "sotyware-device-wrap-v1".into(),
            account: account.into(),
            kid: ak.id(),
            key: ak.to_base64().to_string(),
        })
        .expect("serializes"),
    );
    let sealed = crypto_box::PublicKey::from(pk)
        .seal(&mut OsRng, &payload)
        .map_err(|_| CryptoError::Format("seal".into()))?;
    Ok(B64.encode(sealed))
}

/// Opens a sealed `AK` with this device's key; refuses one for another
/// account.
pub fn open_for_device(
    device: &DeviceKey,
    account: &str,
    sealed: &str,
) -> Result<SecretKey, CryptoError> {
    let bytes = B64
        .decode(sealed)
        .map_err(|_| CryptoError::Format("sealed key".into()))?;
    let plain = Zeroizing::new(device.0.unseal(&bytes).map_err(|_| CryptoError::Decrypt)?);
    let p: SealedPayload =
        serde_json::from_slice(&plain).map_err(|e| CryptoError::Format(e.to_string()))?;
    let key = Zeroizing::new(p.key);
    if p.format != "sotyware-device-wrap-v1" || p.account != account {
        return Err(CryptoError::Format("a key for another account".into()));
    }
    let ak = SecretKey::from_base64(&key)?;
    if ak.id() != p.kid {
        return Err(CryptoError::Decrypt);
    }
    Ok(ak)
}

/// The fingerprint both screens show when a device asks to be approved:
/// 15 decimal digits in three groups, from SHA-256(public key ‖ request id).
pub fn fingerprint(public_key: &str, enrollment: &str) -> String {
    let d = Sha256::digest(format!("sotyware enrollment v1|{public_key}|{enrollment}").as_bytes());
    let n = u64::from_be_bytes(d[..8].try_into().expect("8 bytes")) % 1_000_000_000_000_000;
    let s = format!("{n:015}");
    format!("{} {} {}", &s[..5], &s[5..10], &s[10..])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recovery_keys_round_trip_and_catch_typos() {
        let rk = SecretKey::generate();
        let shown = format_recovery_key(&rk);
        assert_eq!(shown.len(), 56 + 13, "{}", *shown);
        assert_eq!(shown.split('-').count(), 14);
        assert_eq!(parse_recovery_key(&shown).unwrap(), rk);
        // Lowercase, spaces, O for 0 and l for 1 are accepted.
        let messy = shown
            .to_lowercase()
            .replace('-', " ")
            .replace('0', "o")
            .replace('1', "l");
        assert_eq!(parse_recovery_key(&messy).unwrap(), rk);
        // One changed character is refused by the checksum.
        let mut chars: Vec<char> = shown.chars().collect();
        chars[3] = if chars[3] == 'A' { 'B' } else { 'A' };
        let typo: String = chars.into_iter().collect();
        assert_eq!(parse_recovery_key(&typo), Err(CryptoError::RecoveryKey));
        assert_eq!(
            parse_recovery_key("too short"),
            Err(CryptoError::RecoveryKey)
        );
    }

    #[test]
    fn the_vault_decrypts_only_with_its_key_account_and_format() {
        let ak = SecretKey::generate();
        let env = encrypt_vault(&ak, "acc-1", b"{\"records\":{}}");
        assert!(!env.contains("records"), "ciphertext only");
        assert_eq!(
            &**decrypt_vault(&ak, "acc-1", &env).unwrap(),
            b"{\"records\":{}}"
        );
        assert_eq!(
            decrypt_vault(&SecretKey::generate(), "acc-1", &env),
            Err(CryptoError::Decrypt)
        );
        assert_eq!(
            decrypt_vault(&ak, "acc-2", &env),
            Err(CryptoError::Decrypt),
            "bound to the account"
        );
        // A changed byte, an older version or another algorithm is refused.
        let mut e: Envelope = serde_json::from_str(&env).unwrap();
        let mut ct = B64.decode(&e.ct).unwrap();
        ct[0] ^= 1;
        e.ct = B64.encode(ct);
        assert_eq!(
            decrypt_vault(&ak, "acc-1", &serde_json::to_string(&e).unwrap()),
            Err(CryptoError::Decrypt)
        );
        let mut old: Envelope = serde_json::from_str(&env).unwrap();
        old.v = 0;
        assert!(matches!(
            decrypt_vault(&ak, "acc-1", &serde_json::to_string(&old).unwrap()),
            Err(CryptoError::Format(_))
        ));
        let mut weak: Envelope = serde_json::from_str(&env).unwrap();
        weak.alg = "none".into();
        assert!(matches!(
            decrypt_vault(&ak, "acc-1", &serde_json::to_string(&weak).unwrap()),
            Err(CryptoError::Format(_))
        ));
        // Fresh nonces: the same plaintext never gives the same ciphertext.
        assert_ne!(
            encrypt_vault(&ak, "acc-1", b"x"),
            encrypt_vault(&ak, "acc-1", b"x")
        );
    }

    #[test]
    fn the_account_key_unwraps_with_the_recovery_key_only() {
        let (ak, rk) = (SecretKey::generate(), SecretKey::generate());
        let wrap = wrap_with_recovery(&ak, &rk, "acc");
        assert!(!wrap.contains(&*ak.to_base64()));
        assert_eq!(unwrap_with_recovery(&wrap, &rk, "acc").unwrap(), ak);
        assert_eq!(
            unwrap_with_recovery(&wrap, &SecretKey::generate(), "acc"),
            Err(CryptoError::Decrypt)
        );
        assert_eq!(
            unwrap_with_recovery(&wrap, &rk, "other"),
            Err(CryptoError::Decrypt)
        );
    }

    #[test]
    fn devices_open_only_their_own_sealed_key() {
        let ak = SecretKey::generate();
        let (a, b) = (DeviceKey::generate(), DeviceKey::generate());
        let sealed = seal_for_device(&ak, "acc", &a.public_base64()).unwrap();
        assert_eq!(open_for_device(&a, "acc", &sealed).unwrap(), ak);
        assert_eq!(
            open_for_device(&b, "acc", &sealed),
            Err(CryptoError::Decrypt)
        );
        assert!(
            open_for_device(&a, "other", &sealed).is_err(),
            "bound to the account"
        );
        // The secret half survives a round trip through the credential store.
        let again =
            DeviceKey::from_secret(&SecretKey::from_base64(&a.secret().to_base64()).unwrap());
        assert_eq!(again.public_base64(), a.public_base64());
    }

    #[test]
    fn proofs_and_fingerprints_are_stable_and_one_way() {
        let ak = SecretKey::generate();
        assert_eq!(key_proof(&ak, "acc"), key_proof(&ak, "acc"));
        assert_ne!(key_proof(&ak, "acc"), key_proof(&ak, "acc2"));
        assert_ne!(key_proof(&ak, "acc"), B64.encode(ak.bytes()));
        let f = fingerprint("pk", "req");
        assert_eq!(f.len(), 17);
        assert_eq!(f, fingerprint("pk", "req"));
        assert_ne!(f, fingerprint("pk2", "req"));
        assert_eq!(format!("{:?}", ak), "SecretKey(…)", "never printed");
    }
}
