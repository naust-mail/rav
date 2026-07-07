/// Link proxy: sign, verify, and rewrite email href links so they pass through
/// /api/v1/link before reaching the destination.
///
/// Each proxy URL is HMAC-SHA256 signed over (url, user_hash, exp) so:
///   - Only the server can produce valid links (no open redirect)
///   - A link signed for user A is rejected under user B's session
///   - Links expire after 30 days
///
/// Excluded from proxying: mailto:, cid:, tel:, sms:, #anchors,
/// javascript:, data:, vbscript:, and empty hrefs.
use std::fmt::Write as FmtWrite;
use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine;
use hmac::{KeyInit, Mac, SimpleHmac};
use rand::RngCore;
use sha2::Sha256;

/// Wraps the link proxy secret so it can be passed as an Axum Extension.
/// Only present when LINK_PROXY_ENABLED=true.
#[derive(Clone)]
pub struct LinkProxySecret(pub Vec<u8>);

type HmacSha256 = SimpleHmac<Sha256>;

const EXPIRY_SECS: u64 = 30 * 24 * 3600;
const TAG_BYTES: usize = 16;

/// Generate or load the 32-byte proxy secret from `{data_dir}/link_proxy.key`.
/// Creates and persists the file on first run.
pub fn load_or_create_secret(data_dir: &str) -> Result<Vec<u8>, String> {
    let path = std::path::Path::new(data_dir).join("link_proxy.key");

    if path.exists() {
        let hex = std::fs::read_to_string(&path)
            .map_err(|e| format!("Failed to read link_proxy.key: {e}"))?;
        let bytes = decode_hex(hex.trim())
            .map_err(|e| format!("link_proxy.key is corrupt: {e}"))?;
        if bytes.len() != 32 {
            return Err("link_proxy.key must be 32 bytes (64 hex chars)".into());
        }
        return Ok(bytes);
    }

    let mut secret = [0u8; 32];
    rand::rng().fill_bytes(secret.as_mut_slice());
    let hex = encode_hex(&secret);
    std::fs::write(&path, &hex)
        .map_err(|e| format!("Failed to write link_proxy.key: {e}"))?;
    tracing::info!("Generated new link proxy signing key at {}", path.display());
    Ok(secret.to_vec())
}

/// Rewrite all `http(s)://` hrefs in the given HTML to proxy URLs.
/// `base` is the scheme+host to prepend, e.g. `"https://mail.example.com"`.
pub fn rewrite_html_links(html: &str, secret: &[u8], user_hash: &str, base: &str) -> String {
    let exp = now_secs() + EXPIRY_SECS;

    // Match href="..." and href='...' (both quote styles)
    let mut out = String::with_capacity(html.len() + 64);
    let mut rest = html;

    while let Some(pos) = find_href(rest) {
        out.push_str(&rest[..pos.start]);

        let raw = &rest[pos.href_start..pos.href_end];
        let quote = pos.quote;

        if should_proxy(raw) {
            let signed = sign_url(secret, raw, user_hash, exp as i64);
            let _ = write!(out, "href={quote}{base}/api/v1/link?{signed}{quote}");
        } else {
            let _ = write!(out, "href={quote}{raw}{quote}");
        }

        rest = &rest[pos.href_end + 1..]; // +1 to skip closing quote
    }

    out.push_str(rest);
    out
}

/// Build the HMAC-signed query string for a single URL.
/// Returns `u=<b64url>&exp=<exp>&s=<hex_tag>`.
pub fn sign_url(secret: &[u8], url: &str, user_hash: &str, exp: i64) -> String {
    let tag = compute_mac(secret, url, user_hash, exp);
    let u = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(url.as_bytes());
    format!("u={u}&exp={exp}&s={}", encode_hex(&tag))
}

/// Verify the query params from /api/v1/link. Returns the destination URL on success.
pub fn verify(
    secret: &[u8],
    u: &str,
    exp: i64,
    s: &str,
    user_hash: &str,
) -> Result<String, &'static str> {
    let now = now_secs() as i64;
    if exp < now {
        return Err("link expired");
    }

    let url_bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(u)
        .map_err(|_| "invalid url encoding")?;
    let url = String::from_utf8(url_bytes).map_err(|_| "url is not utf-8")?;

    let expected = compute_mac(secret, &url, user_hash, exp);
    let provided = decode_hex(s).map_err(|_| "invalid signature encoding")?;

    if provided.len() != TAG_BYTES || !constant_eq(&expected, &provided) {
        return Err("invalid signature");
    }

    // Final sanity: only redirect to http(s) destinations
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Err("non-http destination");
    }

    Ok(url)
}

fn compute_mac(secret: &[u8], url: &str, user_hash: &str, exp: i64) -> Vec<u8> {
    let mut mac = HmacSha256::new_from_slice(secret).expect("HMAC accepts any key length");
    mac.update(url.as_bytes());
    mac.update(b"\n");
    mac.update(user_hash.as_bytes());
    mac.update(b"\n");
    mac.update(exp.to_be_bytes().as_ref());
    mac.finalize().into_bytes()[..TAG_BYTES].to_vec()
}

fn should_proxy(href: &str) -> bool {
    let v = href.trim();
    if v.is_empty() || v.starts_with('#') {
        return false;
    }
    let scheme = match v.find(':') {
        Some(i) => v[..i].to_lowercase(),
        None => return false,
    };
    scheme == "http" || scheme == "https"
}

struct HrefPos {
    start: usize,
    quote: char,
    href_start: usize,
    href_end: usize,
}

/// Find the next `href=` attribute in `s`, return byte offsets.
fn find_href(s: &str) -> Option<HrefPos> {
    let bytes = s.as_bytes();
    let mut i = 0;

    while i + 5 < bytes.len() {
        // Case-insensitive search for "href="
        if bytes[i..].len() >= 5
            && bytes[i].eq_ignore_ascii_case(&b'h')
            && bytes[i + 1].eq_ignore_ascii_case(&b'r')
            && bytes[i + 2].eq_ignore_ascii_case(&b'e')
            && bytes[i + 3].eq_ignore_ascii_case(&b'f')
            && bytes[i + 4] == b'='
        {
            let after_eq = i + 5;
            if after_eq >= bytes.len() {
                break;
            }
            let q = bytes[after_eq];
            if q == b'"' || q == b'\'' {
                let href_start = after_eq + 1;
                if let Some(end_off) = bytes[href_start..].iter().position(|&b| b == q) {
                    return Some(HrefPos {
                        start: i,
                        quote: q as char,
                        href_start,
                        href_end: href_start + end_off,
                    });
                }
            }
        }
        i += 1;
    }
    None
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn encode_hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(s, "{b:02x}");
    }
    s
}

fn decode_hex(s: &str) -> Result<Vec<u8>, String> {
    if !s.len().is_multiple_of(2) {
        return Err("odd hex length".into());
    }
    (0..s.len() / 2)
        .map(|i| u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).map_err(|e| e.to_string()))
        .collect()
}

/// Constant-time equality for equal-length slices.
fn constant_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}
