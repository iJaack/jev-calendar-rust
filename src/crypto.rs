use aes_gcm::{Aes256Gcm, KeyInit, Nonce, aead::Aead};
use anyhow::{Result, anyhow, ensure};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD as B64};
use hmac::{Hmac, Mac};
use rand::RngCore;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

pub fn random(bytes: usize) -> String {
    let mut b = vec![0; bytes];
    rand::rng().fill_bytes(&mut b);
    B64.encode(b)
}
pub fn sign(secret: &str, payload: &str) -> String {
    let mut m =
        <Hmac<Sha256> as Mac>::new_from_slice(secret.as_bytes()).expect("HMAC accepts any key");
    m.update(payload.as_bytes());
    B64.encode(m.finalize().into_bytes())
}
pub fn verify(secret: &str, payload: &str, signature: &str) -> Result<()> {
    let bytes = B64
        .decode(signature)
        .map_err(|_| anyhow!("Invalid signature"))?;
    let mut m =
        <Hmac<Sha256> as Mac>::new_from_slice(secret.as_bytes()).expect("HMAC accepts any key");
    m.update(payload.as_bytes());
    m.verify_slice(&bytes)
        .map_err(|_| anyhow!("Invalid signature"))
}
pub struct SecretBox {
    cipher: Aes256Gcm,
}
impl SecretBox {
    pub fn new(secret: &str) -> Self {
        Self {
            cipher: Aes256Gcm::new(&Sha256::digest(secret.as_bytes())),
        }
    }
    pub fn encrypt(&self, value: &str) -> Result<String> {
        let mut iv = [0u8; 12];
        rand::rng().fill_bytes(&mut iv);
        let encrypted = self
            .cipher
            .encrypt(Nonce::from_slice(&iv), value.as_bytes())
            .map_err(|_| anyhow!("Encryption failed"))?;
        let n = encrypted.len() - 16;
        let mut out = iv.to_vec();
        out.extend_from_slice(&encrypted[n..]);
        out.extend_from_slice(&encrypted[..n]);
        Ok(B64.encode(out))
    }
    pub fn decrypt(&self, value: &str) -> Result<String> {
        let raw = B64.decode(value)?;
        ensure!(raw.len() >= 28, "Invalid encrypted token");
        let mut encrypted = raw[28..].to_vec();
        encrypted.extend_from_slice(&raw[12..28]);
        let plain = self
            .cipher
            .decrypt(Nonce::from_slice(&raw[..12]), encrypted.as_slice())
            .map_err(|_| anyhow!("Invalid encrypted token"))?;
        Ok(String::from_utf8(plain)?)
    }
}
pub fn management_token(secret: &str, key: &str) -> String {
    let payload = B64.encode(key);
    format!(
        "{payload}.{}",
        sign(secret, &format!("jev-booking-management-v1:{payload}"))
    )
}
pub fn management_key(secret: &str, token: &str) -> Result<String> {
    let decode = || -> Result<String> {
        ensure!(token.len() <= 2048, "Invalid token");
        let parts: Vec<_> = token.split('.').collect();
        ensure!(
            parts.len() == 2 && !parts[0].is_empty() && parts[1].len() == 43,
            "Invalid token"
        );
        ensure!(
            parts[0]
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-'),
            "Invalid token"
        );
        verify(
            secret,
            &format!("jev-booking-management-v1:{}", parts[0]),
            parts[1],
        )?;
        Ok(String::from_utf8(B64.decode(parts[0])?)?)
    };
    decode().map_err(|_| {
        anyhow!("This meeting link is invalid. Open the link from your calendar invitation.")
    })
}
pub fn management_urls(secret: &str, origin: &str, key: &str) -> Value {
    let token = management_token(secret, key);
    let mut urls = json!({});
    for action in ["view", "cancel", "reschedule"] {
        let query = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("token", &token)
            .append_pair("action", action)
            .finish();
        urls[action] = json!(format!("{origin}/manage#{query}"));
    }
    urls
}
pub fn address_encode(secret: &str, selection: &Value) -> String {
    let payload=B64.encode(json!({"provider":"photon","providerId":selection["providerId"],"formattedAddress":selection["text"],"latitude":selection["latitude"],"longitude":selection["longitude"]}).to_string());
    format!("{payload}.{}", sign(secret, &payload))
}
pub fn address_decode(secret: &str, token: &str) -> Result<Value> {
    let decode = || -> Result<Value> {
        let p: Vec<_> = token.split('.').collect();
        ensure!(
            p.len() == 2 && !p[0].is_empty() && p[0].len() <= 2000,
            "Invalid address"
        );
        verify(secret, p[0], p[1])?;
        let v: Value = serde_json::from_slice(&B64.decode(p[0])?)?;
        ensure!(
            v["provider"] == "photon"
                && !crate::text(&v, "providerId").is_empty()
                && !crate::text(&v, "formattedAddress").trim().is_empty()
                && v["latitude"].as_f64().is_some()
                && v["longitude"].as_f64().is_some(),
            "Invalid address"
        );
        Ok(
            json!({"provider":"photon","providerId":v["providerId"],"formattedAddress":crate::text(&v,"formattedAddress").split_whitespace().collect::<Vec<_>>().join(" "),"location":{"latitude":v["latitude"],"longitude":v["longitude"]}}),
        )
    };
    decode().map_err(|_| anyhow!("Choose an address from the search results"))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_modified_ciphertext_and_capabilities() {
        let b = SecretBox::new("synthetic-key");
        let c = b.encrypt("private").unwrap();
        assert_eq!(b.decrypt(&c).unwrap(), "private");
        assert!(SecretBox::new("other").decrypt(&c).is_err());
        let t = management_token("secret", "booking-key");
        assert_eq!(management_key("secret", &t).unwrap(), "booking-key");
        assert!(management_key("other", &t).is_err());
        assert!(management_key("secret", &(t + ".extra")).is_err());
    }
}
