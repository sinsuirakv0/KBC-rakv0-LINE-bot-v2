use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use hmac::{Hmac, Mac};
use sha2::Sha256;

// Webと共通のJST 6時間単位・96bit署名。正本の仕様はkbc-web/docs/url-key.md。
pub(crate) fn generate_url_key(secret: &str, now_ms: i64) -> Option<String> {
    if secret.len() < 32 || now_ms < 0 {
        return None;
    }
    let mut bucket = ((now_ms + 32_400_000) / 21_600_000) as u64;
    let mut digits = Vec::new();
    loop {
        digits.push(b"0123456789abcdefghijklmnopqrstuvwxyz"[(bucket % 36) as usize] as char);
        bucket /= 36;
        if bucket == 0 {
            break;
        }
    }
    let stamp: String = digits.into_iter().rev().collect();
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).ok()?;
    mac.update(format!("kbc-urlkey:v1:{stamp}").as_bytes());
    let signature = mac.finalize().into_bytes();
    Some(format!(
        "1.{stamp}.{}",
        URL_SAFE_NO_PAD.encode(&signature[..12])
    ))
}

pub(crate) fn club_link(now_ms: i64) -> Option<String> {
    let secret = std::env::var("URLKEY_SECRET").ok()?;
    let key = generate_url_key(&secret, now_ms)?;
    Some(format!(
        "https://kbc-rakv0.pages.dev/pages/nyanko-club-forgery/?URLkey={key}"
    ))
}

#[cfg(test)]
mod tests {
    #[test]
    fn matches_web_vector() {
        assert_eq!(
            super::generate_url_key("test-secret-for-kbc-urlkey-32-bytes", 1_792_224_000_000)
                .as_deref(),
            Some("1.1s0u.QPHyWhTQYFauKay6")
        );
    }
}
