//! 哈希函数
//!
//! 使用 ring 库实现 SHA-256 和 PBKDF2 密钥派生。

use ring::digest;
use ring::pbkdf2;
use ring::rand::{SecureRandom, SystemRandom};
use std::num::NonZeroU32;

/// 计算 SHA-256 哈希
pub fn sha256(data: &[u8]) -> Vec<u8> {
    let digest = digest::digest(&digest::SHA256, data);
    digest.as_ref().to_vec()
}

/// 计算 SHA-256 哈希并返回十六进制字符串
pub fn sha256_hex(data: &[u8]) -> String {
    let hash = sha256(data);
    hex_encode(&hash)
}

/// 计算 SHA-256 SSH 密钥指纹（Base64 编码）
pub fn sha256_fingerprint(data: &[u8]) -> String {
    let hash = sha256(data);
    format!("SHA256:{}", base64_encode(&hash))
}

/// PBKDF2-HMAC-SHA256 密钥派生
///
/// 从密码和盐派生 32 字节密钥。
/// 迭代次数至少 100,000 次。
pub fn derive_key(password: &str, salt: &[u8], iterations: u32) -> [u8; 32] {
    let mut derived = [0u8; 32];
    let iterations = NonZeroU32::new(iterations).unwrap_or(NonZeroU32::new(100_000).unwrap());

    pbkdf2::derive(
        pbkdf2::PBKDF2_HMAC_SHA256,
        iterations,
        salt,
        password.as_bytes(),
        &mut derived,
    );

    derived
}

/// 生成随机盐
pub fn generate_salt() -> [u8; 16] {
    let mut salt = [0u8; 16];
    let rng = SystemRandom::new();
    rng.fill(&mut salt).expect("Failed to generate random salt");
    salt
}

/// 验证 PBKDF2 派生密钥
pub fn verify_key(password: &str, salt: &[u8], iterations: u32, expected: &[u8; 32]) -> bool {
    let derived = derive_key(password, salt, iterations);
    // 常量时间比较
    #[allow(deprecated)]
    ring::constant_time::verify_slices_are_equal(&derived, expected).is_ok()
}

/// 十六进制编码（公开：主密码持久化文件用 hex 承载盐与令牌）
pub fn hex_encode(data: &[u8]) -> String {
    data.iter().map(|b| format!("{:02x}", b)).collect()
}

/// 十六进制解码
///
/// 奇数长度或非 hex 字符一律报错，不做静默截断——持久化文件被篡改时
/// 必须显式失败，不能解出错误的长度后继续用。
pub fn hex_decode(text: &str) -> Result<Vec<u8>, String> {
    if !text.len().is_multiple_of(2) {
        return Err(format!("hex string has odd length: {}", text.len()));
    }
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len() / 2);
    for pair in bytes.chunks(2) {
        let hi = hex_val(pair[0])?;
        let lo = hex_val(pair[1])?;
        out.push((hi << 4) | lo);
    }
    Ok(out)
}

fn hex_val(byte: u8) -> Result<u8, String> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        other => Err(format!("invalid hex character: {}", other as char)),
    }
}

/// Base64 编码（标准，无填充）
fn base64_encode(data: &[u8]) -> String {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut result = String::new();

    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };

        let triple = (b0 << 16) | (b1 << 8) | b2;

        result.push(CHARS[((triple >> 18) & 0x3F) as usize] as char);
        result.push(CHARS[((triple >> 12) & 0x3F) as usize] as char);

        if chunk.len() > 1 {
            result.push(CHARS[((triple >> 6) & 0x3F) as usize] as char);
        }
        if chunk.len() > 2 {
            result.push(CHARS[(triple & 0x3F) as usize] as char);
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sha256() {
        let hash = sha256(b"hello");
        assert_eq!(hash.len(), 32);
    }

    #[test]
    fn test_sha256_hex() {
        let hex = sha256_hex(b"hello");
        assert_eq!(hex.len(), 64);
    }

    #[test]
    fn test_pbkdf2_derive_and_verify() {
        let password = "my_password";
        let salt = generate_salt();
        let iterations = 100_000;

        let key = derive_key(password, &salt, iterations);
        assert!(verify_key(password, &salt, iterations, &key));
        assert!(!verify_key("wrong_password", &salt, iterations, &key));
    }

    #[test]
    fn test_fingerprint() {
        let fp = sha256_fingerprint(b"test_key_data");
        assert!(fp.starts_with("SHA256:"));
    }
}
