use aes_gcm::{
    Aes256Gcm, KeyInit, Nonce,
    aead::{Aead, OsRng, rand_core::RngCore},
};
use argon2::{
    Argon2, PasswordHash, PasswordHasher, PasswordVerifier,
    password_hash::{SaltString, rand_core::OsRng as PasswordOsRng},
};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use sha2::{Digest, Sha256};

use crate::error::{ApiError, ApiResult};

#[derive(Clone)]
pub struct Crypto {
    cipher: Aes256Gcm,
}

impl Crypto {
    pub fn new(key: &[u8; 32]) -> Self {
        Self {
            cipher: Aes256Gcm::new_from_slice(key).expect("32-byte AES key"),
        }
    }

    pub fn encrypt(&self, plaintext: &[u8]) -> ApiResult<String> {
        let mut nonce = [0u8; 12];
        OsRng.fill_bytes(&mut nonce);
        let ciphertext = self
            .cipher
            .encrypt(Nonce::from_slice(&nonce), plaintext)
            .map_err(|_| ApiError::internal("credential encryption failed"))?;
        let mut result = nonce.to_vec();
        result.extend(ciphertext);
        Ok(STANDARD.encode(result))
    }

    pub fn decrypt(&self, encoded: &str) -> ApiResult<Vec<u8>> {
        let value = STANDARD
            .decode(encoded)
            .map_err(|_| ApiError::internal("stored credential is malformed"))?;
        if value.len() < 13 {
            return Err(ApiError::internal("stored credential is malformed"));
        }
        self.cipher
            .decrypt(Nonce::from_slice(&value[..12]), &value[12..])
            .map_err(|_| ApiError::internal("credential decryption failed"))
    }
}

pub fn hash_password(password: &str) -> ApiResult<String> {
    let salt = SaltString::generate(&mut PasswordOsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|value| value.to_string())
        .map_err(|_| ApiError::internal("password hashing failed"))
}

pub fn verify_password(password: &str, encoded: &str) -> bool {
    PasswordHash::new(encoded)
        .ok()
        .and_then(|hash| {
            Argon2::default()
                .verify_password(password.as_bytes(), &hash)
                .ok()
        })
        .is_some()
}

pub fn token_hash(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

pub fn random_token(bytes: usize) -> ApiResult<String> {
    let mut value = vec![0u8; bytes];
    getrandom::fill(&mut value).map_err(|_| ApiError::internal("random generation failed"))?;
    Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(value))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credentials_round_trip_and_wrong_key_fails() {
        let crypto = Crypto::new(&[7; 32]);
        let encrypted = crypto.encrypt(b"secret").unwrap();
        assert_eq!(crypto.decrypt(&encrypted).unwrap(), b"secret");
        assert!(Crypto::new(&[8; 32]).decrypt(&encrypted).is_err());
    }

    #[test]
    fn password_hashes_and_verifies() {
        let hash = hash_password("correct horse").unwrap();
        assert!(verify_password("correct horse", &hash));
        assert!(!verify_password("wrong", &hash));
    }

    #[test]
    fn token_hash_is_stable_without_storing_plaintext() {
        let hash = token_hash("sk-mini_test");
        assert_eq!(hash, token_hash("sk-mini_test"));
        assert!(!hash.contains("sk-mini"));
    }
}
