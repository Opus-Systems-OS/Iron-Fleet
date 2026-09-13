//! Standard Webhooks signature verification — what the Anthropic SDKs'
//! `client.beta.webhooks.unwrap()` does. There is no Rust SDK, so this is the
//! port, checked against the `standardwebhooks` reference library:
//!
//! - headers: `webhook-id`, `webhook-timestamp`, `webhook-signature`
//! - secret: `whsec_` + base64 (decoded to raw key bytes)
//! - signed string: `"{id}.{timestamp}.{raw body}"`, HMAC-SHA256
//! - header value: space-separated `v1,<base64 mac>` entries; any `v1` may match
//! - timestamp: unix seconds, must be within ±5 minutes of now

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use subtle::ConstantTimeEq;

pub const TOLERANCE_SECS: i64 = 5 * 60;

#[derive(Clone)]
pub struct SigningKey(Vec<u8>);

impl std::fmt::Debug for SigningKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SigningKey(..)")
    }
}

impl SigningKey {
    /// Accepts the `whsec_…` value shown once in the Console.
    pub fn parse(secret: &str) -> Result<Self, &'static str> {
        let b64 = secret.strip_prefix("whsec_").unwrap_or(secret);
        // The reference implementation appends "==" and lets the decoder cope
        // with excess padding; do the equivalent by trying both.
        let bytes = B64
            .decode(b64)
            .or_else(|_| B64.decode(format!("{b64}==")))
            .map_err(|_| "signing key is not valid base64")?;
        if bytes.is_empty() {
            return Err("signing key is empty");
        }
        Ok(SigningKey(bytes))
    }
}

pub struct Headers<'a> {
    pub id: &'a str,
    pub timestamp: &'a str,
    pub signature: &'a str,
}

/// Verify a delivery. Returns the message id on success.
pub fn verify(
    key: &SigningKey,
    headers: Headers<'_>,
    body: &[u8],
    now_unix: i64,
) -> Result<(), &'static str> {
    if headers.id.is_empty() {
        return Err("missing webhook-id");
    }
    let ts: i64 = headers
        .timestamp
        .parse()
        .map_err(|_| "webhook-timestamp is not an integer")?;
    if ts < now_unix - TOLERANCE_SECS {
        return Err("webhook-timestamp too old");
    }
    if ts > now_unix + TOLERANCE_SECS {
        return Err("webhook-timestamp too far in the future");
    }

    let expected = sign(key, headers.id, headers.timestamp, body);

    let mut saw_v1 = false;
    for entry in headers.signature.split(' ').filter(|e| !e.is_empty()) {
        let Some((version, sig)) = entry.split_once(',') else {
            continue;
        };
        if version != "v1" {
            continue;
        }
        saw_v1 = true;
        let Ok(sig) = B64.decode(sig) else { continue };
        if sig.len() == expected.len() && bool::from(sig.ct_eq(&expected)) {
            return Ok(());
        }
    }
    Err(if saw_v1 {
        "no matching v1 signature"
    } else {
        "no v1 signature present"
    })
}

pub fn sign(key: &SigningKey, id: &str, timestamp: &str, body: &[u8]) -> Vec<u8> {
    let mut mac = Hmac::<Sha256>::new_from_slice(&key.0).expect("HMAC accepts any key length");
    mac.update(id.as_bytes());
    mac.update(b".");
    mac.update(timestamp.as_bytes());
    mac.update(b".");
    mac.update(body);
    mac.finalize().into_bytes().to_vec()
}

/// Header value for a signed test delivery: `v1,<base64>`.
pub fn signature_header(key: &SigningKey, id: &str, timestamp: &str, body: &[u8]) -> String {
    format!("v1,{}", B64.encode(sign(key, id, timestamp, body)))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Vector from the Standard Webhooks spec test suite:
    // secret whsec_MfKQ9r8GKYqrTwjUPD8ILPZIo2LaLaSw, id msg_p5jXN8AQM9LWM0D4loKWxJek,
    // timestamp 1614265330, payload {"test": 2432232314}
    // -> v1,g0hM9SsE+OTPJTGt/tmIKtSyZlE3uFJELVlNIOLJ1OE=
    const SECRET: &str = "whsec_MfKQ9r8GKYqrTwjUPD8ILPZIo2LaLaSw";
    const ID: &str = "msg_p5jXN8AQM9LWM0D4loKWxJek";
    const TS: &str = "1614265330";
    const BODY: &[u8] = br#"{"test": 2432232314}"#;
    const SIG: &str = "v1,g0hM9SsE+OTPJTGt/tmIKtSyZlE3uFJELVlNIOLJ1OE=";

    fn key() -> SigningKey {
        SigningKey::parse(SECRET).unwrap()
    }

    #[test]
    fn matches_reference_vector() {
        assert_eq!(signature_header(&key(), ID, TS, BODY), SIG);
        assert!(verify(
            &key(),
            Headers {
                id: ID,
                timestamp: TS,
                signature: SIG
            },
            BODY,
            1614265330
        )
        .is_ok());
    }

    #[test]
    fn accepts_any_matching_v1_among_several() {
        let sig = format!("v1,AAAA v2,zzzz {SIG}");
        assert!(verify(
            &key(),
            Headers {
                id: ID,
                timestamp: TS,
                signature: &sig
            },
            BODY,
            1614265330
        )
        .is_ok());
    }

    #[test]
    fn rejects_tampered_body_and_wrong_key() {
        let mut body = BODY.to_vec();
        body[2] ^= 1;
        assert_eq!(
            verify(
                &key(),
                Headers {
                    id: ID,
                    timestamp: TS,
                    signature: SIG
                },
                &body,
                1614265330
            ),
            Err("no matching v1 signature")
        );
        let other =
            SigningKey::parse("whsec_AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=").unwrap();
        assert!(verify(
            &other,
            Headers {
                id: ID,
                timestamp: TS,
                signature: SIG
            },
            BODY,
            1614265330
        )
        .is_err());
    }

    #[test]
    fn rejects_stale_and_future_timestamps() {
        let ok = |now| {
            verify(
                &key(),
                Headers {
                    id: ID,
                    timestamp: TS,
                    signature: SIG,
                },
                BODY,
                now,
            )
        };
        assert!(ok(1614265330 + TOLERANCE_SECS).is_ok());
        assert_eq!(
            ok(1614265330 + TOLERANCE_SECS + 1),
            Err("webhook-timestamp too old")
        );
        assert_eq!(
            ok(1614265330 - TOLERANCE_SECS - 1),
            Err("webhook-timestamp too far in the future")
        );
    }

    #[test]
    fn rejects_missing_or_non_v1() {
        assert_eq!(
            verify(
                &key(),
                Headers {
                    id: ID,
                    timestamp: TS,
                    signature: "v2,abc"
                },
                BODY,
                1614265330
            ),
            Err("no v1 signature present")
        );
        assert_eq!(
            verify(
                &key(),
                Headers {
                    id: "",
                    timestamp: TS,
                    signature: SIG
                },
                BODY,
                1614265330
            ),
            Err("missing webhook-id")
        );
        assert!(verify(
            &key(),
            Headers {
                id: ID,
                timestamp: "abc",
                signature: SIG
            },
            BODY,
            1614265330
        )
        .is_err());
    }

    #[test]
    fn parses_secret_with_and_without_prefix() {
        assert!(SigningKey::parse(SECRET).is_ok());
        assert!(SigningKey::parse(&SECRET["whsec_".len()..]).is_ok());
        assert!(SigningKey::parse("whsec_!!!").is_err());
    }
}
