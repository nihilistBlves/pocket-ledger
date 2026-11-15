//! Enable Banking (<https://enablebanking.com>) HTTP client.
//!
//! Security notes:
//! - The application's RSA private key is read from disk only inside [`jwt`] while
//!   signing, never cached, logged or returned. Only the short-lived signed token is
//!   cached by [`EbClient`].
//! - Secret-bearing values ([`SessionId`], [`AuthCode`], [`State`]) and personal data
//!   ([`Sensitive`]) never show up in `Debug` output; errors never carry URLs, paths,
//!   response bodies or messages.
//! - Logging uses the `log` facade with method, path template, status, attempt and
//!   duration only.

mod client;
mod config;
mod error;
mod jwt;
mod retry;
mod types;

#[cfg(test)]
mod tests;

pub use client::EbClient;
pub use config::{EbConfig, DEFAULT_BASE_URL, KEY_PATH_ENV, REDIRECT_URL};
pub use error::{ApiErrorCode, EbError, NetworkKind};
pub use retry::RetryPolicy;
pub use types::{
    Access, Account, AccountId, Amount, Aspsp, AuthCode, AuthRequest, AuthResponse, Balance,
    CreditDebitIndicator, Party, PsuType, Sensitive, Session, SessionId, SessionInfo, State, Transaction,
    TransactionStatus,
};
