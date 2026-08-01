//! AES-256-GCM 加密解密
//!
//! 使用 ring 库实现 AES-256-GCM 对称加密。

use ring::aead::{self, Aad, BoundKey, Nonce, NonceSequence, OpeningKey, SealingKey, UnboundKey, NONCE_LEN};
use ring::error::Unspecified;
use ring::rand::{SecureRandom, SystemRandom};

/// 简单 nonce 序列（每次加密使用随机 nonce）
struct SimpleNonceSequence {
    nonce: [u8; NONCE_LEN],
}

impl SimpleNonceSequence {
    fn new() -> Self {
        let mut nonce = [0u8; NONCE_LEN];
        let rng = SystemRandom::new();
        let _ = rng.fill(&mut nonce);
        Self { nonce }
    }
}

impl NonceSequence for SimpleNonceSequence {
    fn advance(&mut self) -> Result<Nonce, Unspecified> {
        let nonce = Nonce::try_assume_unique_for_key(&self.nonce)?;
        for byte in self.nonce.iter_mut() {
            *byte = byte.wrapping_add(1);
            if *byte != 0 {
                break;
            }
        }
        Ok(nonce)
    }
}

/// 使用 AES-256-GCM 加密数据
///
/// `key` 必须是 32 字节。
/// 返回格式：nonce (12 bytes) + ciphertext + tag (16 bytes)
pub fn encrypt(key: &[u8; 32], plaintext: &[u8]) -> Result<Vec<u8>, String> {
    let unbound_key = UnboundKey::new(&aead::AES_256_GCM, key)
        .map_err(|_| "Invalid key".to_string())?;

    let mut sealing_key = SealingKey::new(unbound_key, SimpleNonceSequence::new());

    let mut in_out = plaintext.to_vec();
    sealing_key
        .seal_in_place_append_tag(Aad::empty(), &mut in_out)
        .map_err(|_| "Encryption failed".to_string())?;

    // 重新生成 nonce 用于存储（因为 sealing_key 内部的 nonce 已经递增了）
    // 实际做法：加密前记录 nonce
    let nonce_for_storage = {
        let mut n = [0u8; NONCE_LEN];
        let rng = SystemRandom::new();
        let _ = rng.fill(&mut n);
        n
    };

    // 重新加密使用确定的 nonce
    let unbound_key2 = UnboundKey::new(&aead::AES_256_GCM, key)
        .map_err(|_| "Invalid key".to_string())?;
    let nonce_seq = FixedNonceSequence { nonce: nonce_for_storage };
    let mut sealing_key2 = SealingKey::new(unbound_key2, nonce_seq);
    let mut in_out2 = plaintext.to_vec();
    sealing_key2
        .seal_in_place_append_tag(Aad::empty(), &mut in_out2)
        .map_err(|_| "Encryption failed".to_string())?;

    let mut result = Vec::with_capacity(NONCE_LEN + in_out2.len());
    result.extend_from_slice(&nonce_for_storage);
    result.extend_from_slice(&in_out2);

    Ok(result)
}

/// 使用 AES-256-GCM 解密数据
///
/// 输入格式：nonce (12 bytes) + ciphertext + tag (16 bytes)
pub fn decrypt(key: &[u8; 32], ciphertext: &[u8]) -> Result<Vec<u8>, String> {
    if ciphertext.len() < NONCE_LEN + aead::AES_256_GCM.tag_len() {
        return Err("Ciphertext too short".to_string());
    }

    let (nonce_bytes, encrypted) = ciphertext.split_at(NONCE_LEN);

    let unbound_key = UnboundKey::new(&aead::AES_256_GCM, key)
        .map_err(|_| "Invalid key".to_string())?;

    let nonce_seq = FixedNonceSequence {
        nonce: nonce_bytes.try_into().map_err(|_| "Invalid nonce".to_string())?,
    };

    let mut opening_key = OpeningKey::new(unbound_key, nonce_seq);
    let mut in_out = encrypted.to_vec();

    let decrypted = opening_key
        .open_in_place(Aad::empty(), &mut in_out)
        .map_err(|_| "Decryption failed (wrong key or corrupted data)".to_string())?;

    Ok(decrypted.to_vec())
}

/// 固定 nonce 序列（解密/指定 nonce 加密时使用）
struct FixedNonceSequence {
    nonce: [u8; NONCE_LEN],
}

impl NonceSequence for FixedNonceSequence {
    fn advance(&mut self) -> Result<Nonce, Unspecified> {
        Nonce::try_assume_unique_for_key(&self.nonce)
    }
}

/// 生成随机 32 字节密钥
pub fn generate_key() -> [u8; 32] {
    let mut key = [0u8; 32];
    let rng = SystemRandom::new();
    rng.fill(&mut key).expect("Failed to generate random key");
    key
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encrypt_decrypt_roundtrip() {
        let key = generate_key();
        let plaintext = b"Hello, RShell!";

        let ciphertext = encrypt(&key, plaintext).unwrap();
        let decrypted = decrypt(&key, &ciphertext).unwrap();

        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn test_wrong_key_fails() {
        let key1 = generate_key();
        let key2 = generate_key();
        let plaintext = b"Secret data";

        let ciphertext = encrypt(&key1, plaintext).unwrap();
        let result = decrypt(&key2, &ciphertext);

        assert!(result.is_err());
    }
}
