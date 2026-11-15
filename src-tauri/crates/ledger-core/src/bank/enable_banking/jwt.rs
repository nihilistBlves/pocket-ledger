use std::io::ErrorKind;

use jsonwebtoken::{Algorithm, EncodingKey, Header};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use super::config::EbConfig;
use super::error::EbError;

pub(crate) const ISSUER: &str = "enablebanking.com";
pub(crate) const AUDIENCE: &str = "api.enablebanking.com";
/// Lifetime of a minted token.
pub(crate) const TTL_SECS: i64 = 600;

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct Claims {
    pub iss: String,
    pub aud: String,
    pub iat: i64,
    pub exp: i64,
}

/// A signed token and its expiry (unix seconds).
pub(crate) struct SignedToken {
    pub token: Zeroizing<String>,
    pub exp: i64,
}

/// Signs a fresh JWT. The private key file is read here, turned into an encoding
/// key, used and dropped; it is never stored, logged or returned. Error values carry
/// neither the path nor any file content.
pub(crate) fn sign(config: &EbConfig, iat: i64) -> Result<SignedToken, EbError> {
    let pem: Zeroizing<Vec<u8>> = Zeroizing::new(std::fs::read(&config.key_path).map_err(|e| {
        if e.kind() == ErrorKind::NotFound {
            EbError::KeyFileNotFound
        } else {
            EbError::KeyInvalid
        }
    })?);
    let key = EncodingKey::from_rsa_pem(&pem).map_err(|_| EbError::KeyInvalid)?;

    let mut header = Header::new(Algorithm::RS256); // typ = "JWT"
    header.kid = Some(config.application_id.clone());
    let exp = iat + TTL_SECS;
    let claims = Claims { iss: ISSUER.to_owned(), aud: AUDIENCE.to_owned(), iat, exp };
    let token =
        jsonwebtoken::encode(&header, &claims, &key).map_err(|_| EbError::KeyInvalid)?;
    Ok(SignedToken { token: Zeroizing::new(token), exp })
}
