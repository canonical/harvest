use anyhow::{Context, Result};
use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
use base64::Engine;

pub struct Crypto {
    key: [u8; 32],
}

impl Crypto {
    pub fn from_hex(hex_str: &str) -> Result<Self> {
        let bytes = hex::decode(hex_str.trim())
            .context("user_key_encryption_key must be valid hex")?;
        if bytes.len() != 32 {
            anyhow::bail!(
                "user_key_encryption_key must be exactly 32 bytes (64 hex characters), got {} bytes",
                bytes.len()
            );
        }
        let mut key = [0u8; 32];
        key.copy_from_slice(&bytes);
        Ok(Self { key })
    }

    pub fn encrypt(&self, plaintext: &str) -> Result<(String, String)> {
        let cipher = Aes256Gcm::new_from_slice(&self.key)
            .map_err(|e| anyhow::anyhow!("failed to create cipher: {e}"))?;
        let mut nonce_bytes = [0u8; 12];
        use rand_core::RngCore;
        rand_core::OsRng.fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);
        let ciphertext = cipher
            .encrypt(nonce, plaintext.as_bytes())
            .map_err(|e| anyhow::anyhow!("encryption failed: {e}"))?;
        Ok((
            base64::engine::general_purpose::STANDARD.encode(&ciphertext),
            base64::engine::general_purpose::STANDARD.encode(nonce_bytes),
        ))
    }

    pub fn decrypt(&self, ciphertext_b64: &str, nonce_b64: &str) -> Result<String> {
        let cipher = Aes256Gcm::new_from_slice(&self.key)
            .map_err(|e| anyhow::anyhow!("failed to create cipher: {e}"))?;
        let ciphertext = base64::engine::general_purpose::STANDARD
            .decode(ciphertext_b64)
            .context("invalid ciphertext base64")?;
        let nonce_bytes = base64::engine::general_purpose::STANDARD
            .decode(nonce_b64)
            .context("invalid nonce base64")?;
        if nonce_bytes.len() != 12 {
            anyhow::bail!("nonce must be 12 bytes, got {}", nonce_bytes.len());
        }
        let nonce = Nonce::from_slice(&nonce_bytes);
        let plaintext = cipher
            .decrypt(nonce, ciphertext.as_ref())
            .map_err(|e| anyhow::anyhow!("decryption failed: {e}"))?;
        String::from_utf8(plaintext).context("decrypted key is not valid UTF-8")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_key() -> String {
        "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".to_string()
    }

    #[test]
    fn encrypt_decrypt_round_trip() {
        let crypto = Crypto::from_hex(&test_key()).unwrap();
        let (ct, nonce) = crypto.encrypt("my-secret-api-key").unwrap();
        let pt = crypto.decrypt(&ct, &nonce).unwrap();
        assert_eq!(pt, "my-secret-api-key");
    }

    #[test]
    fn decrypt_with_wrong_key_fails() {
        let crypto1 = Crypto::from_hex(&test_key()).unwrap();
        let crypto2 = Crypto::from_hex(&"fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210".to_string()).unwrap();
        let (ct, nonce) = crypto1.encrypt("secret").unwrap();
        assert!(crypto2.decrypt(&ct, &nonce).is_err());
    }

    #[test]
    fn from_hex_rejects_wrong_length() {
        assert!(Crypto::from_hex("short").is_err());
        assert!(Crypto::from_hex("0123456789abcdef").is_err());
    }

    #[test]
    fn encrypt_produces_different_ciphertexts_for_same_plaintext() {
        let crypto = Crypto::from_hex(&test_key()).unwrap();
        let (ct1, _) = crypto.encrypt("same-key").unwrap();
        let (ct2, _) = crypto.encrypt("same-key").unwrap();
        assert_ne!(ct1, ct2);
    }
}
