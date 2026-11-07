use std::collections::BTreeMap;
use std::time::Instant;

use hyper::StatusCode;
use hyper::header::{AUTHORIZATION, HeaderMap};
use ring::hmac;
use serde::Serialize;
use subtle::ConstantTimeEq;
use uuid::Uuid;
use vespera_core::AgentId;

use super::{HttpError, ServerError, UNAUTHORIZED};

pub(super) const SCHEME: &str = "Vespera-HMAC-SHA256";
const WINDOW_MS: u64 = 30_000;
const MAX_NONCES: usize = 128;

type Secret = [u8; 32];

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Principal {
    Operator,
    Worker(AgentId),
}

struct Credential {
    secret: Secret,
    used: BTreeMap<Uuid, u64>,
}

pub(super) struct Authentication {
    session: Uuid,
    started: Instant,
    credentials: BTreeMap<Principal, Credential>,
}

#[derive(Serialize)]
pub(super) struct Session {
    session_id: Uuid,
    elapsed_ms: u64,
}

impl Authentication {
    pub(super) fn new() -> Self {
        Self {
            session: Uuid::new_v4(),
            started: Instant::now(),
            credentials: BTreeMap::new(),
        }
    }

    pub(super) fn approve(
        &mut self,
        principal: Principal,
        env_name: &str,
    ) -> Result<(), ServerError> {
        let mut chars = env_name.bytes();
        if !chars
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == b'_')
            || !chars.all(|c| c.is_ascii_alphanumeric() || c == b'_')
        {
            return Err(ServerError::Credential);
        }
        let value = std::env::var(env_name).map_err(|_| ServerError::Credential)?;
        let secret = decode_hex(&value).ok_or(ServerError::Credential)?;
        if self
            .credentials
            .values()
            .any(|entry| entry.secret.ct_eq(&secret).into())
        {
            return Err(ServerError::DuplicateCredential);
        }
        self.credentials.insert(
            principal,
            Credential {
                secret,
                used: BTreeMap::new(),
            },
        );
        Ok(())
    }

    pub(super) fn revoke(&mut self, worker: AgentId) {
        self.credentials.remove(&Principal::Worker(worker));
    }

    pub(super) fn session(&self) -> Session {
        Session {
            session_id: self.session,
            elapsed_ms: self.elapsed_ms(),
        }
    }

    fn elapsed_ms(&self) -> u64 {
        self.started.elapsed().as_millis() as u64
    }

    pub(super) fn verify(
        &mut self,
        proof: Proof<'_>,
        path: &str,
        body: &[u8],
    ) -> Result<(), HttpError> {
        self.verify_at(proof, path, body, self.elapsed_ms())
    }

    fn verify_at(
        &mut self,
        proof: Proof<'_>,
        path: &str,
        body: &[u8],
        now: u64,
    ) -> Result<(), HttpError> {
        if proof.session != self.session
            || proof.issued_ms > now
            || now - proof.issued_ms >= WINDOW_MS
        {
            return Err(UNAUTHORIZED);
        }
        let credential = self
            .credentials
            .get_mut(&proof.principal)
            .ok_or(UNAUTHORIZED)?;
        let mut context =
            hmac::Context::with_key(&hmac::Key::new(hmac::HMAC_SHA256, &credential.secret));
        context.update(
            format!(
                "{SCHEME}\nPOST\n{path}\n{}\n{}\n{}\n{}\n",
                proof.principal_text, proof.session, proof.issued_ms, proof.nonce
            )
            .as_bytes(),
        );
        context.update(body);
        let expected = context.sign();
        if !bool::from(expected.as_ref().ct_eq(&proof.mac)) {
            return Err(UNAUTHORIZED);
        }
        credential.used.retain(|_, expires| *expires > now);
        if credential.used.contains_key(&proof.nonce) {
            return Err(UNAUTHORIZED);
        }
        if credential.used.len() == MAX_NONCES {
            return Err(HttpError(StatusCode::TOO_MANY_REQUESTS, "nonce_capacity"));
        }
        credential
            .used
            .insert(proof.nonce, proof.issued_ms + WINDOW_MS);
        Ok(())
    }
}

pub(super) struct Proof<'a> {
    pub(super) principal: Principal,
    principal_text: &'a str,
    session: Uuid,
    issued_ms: u64,
    nonce: Uuid,
    mac: [u8; 32],
}

impl<'a> Proof<'a> {
    pub(super) fn parse(headers: &'a HeaderMap) -> Option<Self> {
        let mut values = headers.get_all(AUTHORIZATION).iter();
        let (scheme, proof) = values.next()?.to_str().ok()?.split_once(' ')?;
        if values.next().is_some() || !scheme.eq_ignore_ascii_case(SCHEME) {
            return None;
        }
        let mut fields = proof.split(':');
        let principal_text = fields.next()?;
        let principal = match principal_text {
            "operator" => Principal::Operator,
            worker => {
                canonical_uuid(worker)?;
                Principal::Worker(worker.parse().ok()?)
            }
        };
        let session = canonical_uuid(fields.next()?)?;
        let issued = fields.next()?;
        let issued_ms: u64 = issued.parse().ok()?;
        if issued != issued_ms.to_string() {
            return None;
        }
        let nonce = canonical_uuid(fields.next()?)?;
        let mac = decode_hex(fields.next()?)?;
        if fields.next().is_some() {
            return None;
        }
        Some(Self {
            principal,
            principal_text,
            session,
            issued_ms,
            nonce,
            mac,
        })
    }
}

fn canonical_uuid(text: &str) -> Option<Uuid> {
    let uuid = Uuid::parse_str(text).ok()?;
    (!uuid.is_nil() && text == uuid.hyphenated().encode_lower(&mut Uuid::encode_buffer()))
        .then_some(uuid)
}

fn decode_hex(text: &str) -> Option<[u8; 32]> {
    if text.len() != 64
        || !text
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return None;
    }
    let mut bytes = [0; 32];
    let (pairs, _) = text.as_bytes().as_chunks::<2>();
    for (byte, digits) in bytes.iter_mut().zip(pairs) {
        *byte = u8::from_str_radix(std::str::from_utf8(digits).ok()?, 16).ok()?;
    }
    Some(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: Secret = [33; 32];
    const PATH: &str = "/v2/operator";
    const BODY: &[u8] = b"{}";

    fn auth() -> Authentication {
        let mut auth = Authentication::new();
        auth.credentials.insert(
            Principal::Operator,
            Credential {
                secret: KEY,
                used: BTreeMap::new(),
            },
        );
        auth
    }

    fn proof(session: Uuid, issued_ms: u64, nonce: Uuid) -> Proof<'static> {
        let bytes = format!(
            "Vespera-HMAC-SHA256\nPOST\n/v2/operator\noperator\n{session}\n{issued_ms}\n{nonce}\n{{}}"
        );
        let key = hmac::Key::new(hmac::HMAC_SHA256, &KEY);
        let tag = hmac::sign(&key, bytes.as_bytes());
        Proof {
            principal: Principal::Operator,
            principal_text: "operator",
            session,
            issued_ms,
            nonce,
            mac: tag.as_ref().try_into().unwrap(),
        }
    }

    #[test]
    fn rejects_proofs_outside_the_session_and_time_window() {
        let mut current = auth();
        let previous = auth();
        let nonce = Uuid::new_v4();
        assert!(
            current
                .verify_at(proof(previous.session, 5, nonce), PATH, BODY, 5)
                .is_err()
        );
        assert!(
            current
                .verify_at(proof(current.session, 5, nonce), PATH, BODY, 4)
                .is_err()
        );
        assert!(
            current
                .verify_at(proof(current.session, 5, nonce), PATH, BODY, 5 + WINDOW_MS)
                .is_err()
        );
        assert!(
            current
                .verify_at(proof(current.session, 5, nonce), PATH, BODY, 4 + WINDOW_MS)
                .is_ok()
        );
    }

    #[test]
    fn full_replay_cache_keeps_live_nonces_until_expiry() {
        let mut auth = auth();
        let first = Uuid::new_v4();
        let issued = 5;
        for nonce in std::iter::once(first).chain((1..MAX_NONCES).map(|_| Uuid::new_v4())) {
            assert!(
                auth.verify_at(proof(auth.session, issued, nonce), PATH, BODY, issued)
                    .is_ok()
            );
        }
        let next = Uuid::new_v4();
        let error = auth
            .verify_at(proof(auth.session, issued, next), PATH, BODY, issued)
            .err()
            .unwrap();
        assert_eq!(error.0, StatusCode::TOO_MANY_REQUESTS);
        let error = auth
            .verify_at(proof(auth.session, issued, first), PATH, BODY, issued)
            .err()
            .unwrap();
        assert_eq!(error.0, StatusCode::UNAUTHORIZED);
        let after_expiry = issued + WINDOW_MS;
        assert!(
            auth.verify_at(
                proof(auth.session, after_expiry, next),
                PATH,
                BODY,
                after_expiry
            )
            .is_ok()
        );
        assert!(
            auth.verify_at(proof(auth.session, issued, first), PATH, BODY, after_expiry)
                .is_err()
        );
    }
}
