use std::fmt;

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

/// Wrapper for personal data: `Debug` prints `[redacted]`, deserializes transparently.
/// There is deliberately no `Display`.
#[derive(Clone, PartialEq, Eq, Deserialize)]
#[serde(transparent)]
pub struct Sensitive<T>(T);

impl<T> Sensitive<T> {
    pub fn new(value: T) -> Self {
        Self(value)
    }
    pub fn expose(&self) -> &T {
        &self.0
    }
    pub fn into_inner(self) -> T {
        self.0
    }
}

impl<T> fmt::Debug for Sensitive<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[redacted]")
    }
}

macro_rules! secret_string {
    ($(#[$m:meta])* $name:ident) => {
        $(#[$m])*
        #[derive(Clone, Deserialize)]
        #[serde(from = "String")]
        pub struct $name(Zeroizing<String>);

        impl From<String> for $name {
            fn from(value: String) -> Self {
                Self::new(value)
            }
        }

        impl $name {
            pub fn new(value: impl Into<String>) -> Self {
                Self(Zeroizing::new(value.into()))
            }
            pub fn expose(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(concat!(stringify!($name), "([redacted])"))
            }
        }
    };
}

secret_string!(
    /// Enable Banking session identifier; grants access to the account data.
    SessionId
);
secret_string!(
    /// Single-use authorization code received on the redirect.
    AuthCode
);
secret_string!(
    /// Anti-CSRF `state` of the authorization flow.
    State
);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PsuType {
    Personal,
    Business,
}

impl PsuType {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            PsuType::Personal => "personal",
            PsuType::Business => "business",
        }
    }
}

/// A bank as listed by `GET /aspsps`.
#[derive(Debug, Clone, Deserialize)]
pub struct Aspsp {
    pub name: String,
    pub country: String,
    /// Maximum consent validity in seconds.
    #[serde(default)]
    pub maximum_consent_validity: Option<u64>,
    #[serde(default)]
    pub psu_types: Vec<PsuType>,
}

#[derive(Debug, Clone)]
pub struct AuthRequest {
    pub aspsp_name: String,
    pub aspsp_country: String,
    pub valid_until: DateTime<Utc>,
    pub state: State,
    pub redirect_url: String,
    pub psu_type: PsuType,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AuthResponse {
    /// Bank authorization URL to open in the system browser.
    pub url: Sensitive<String>,
    pub authorization_id: Sensitive<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Access {
    pub valid_until: DateTime<Utc>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AccountId {
    #[serde(default)]
    pub iban: Option<Sensitive<String>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Account {
    /// Only valid while the session is authorized.
    pub uid: Sensitive<String>,
    #[serde(default)]
    pub account_id: Option<AccountId>,
    #[serde(default)]
    pub identification_hash: Option<Sensitive<String>>,
    #[serde(default)]
    pub currency: Option<String>,
    #[serde(default)]
    pub name: Option<Sensitive<String>>,
    #[serde(default)]
    pub details: Option<Sensitive<String>>,
    #[serde(default)]
    pub product: Option<String>,
    #[serde(default)]
    pub cash_account_type: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Session {
    pub session_id: SessionId,
    #[serde(default)]
    pub accounts: Vec<Account>,
    pub access: Access,
}

/// Response of `GET /sessions/{session_id}`; unlike `POST /sessions` it carries
/// account uids only and no session id.
#[derive(Debug, Clone, Deserialize)]
pub struct SessionInfo {
    /// e.g. `AUTHORIZED`, `EXPIRED`, `REVOKED`, `CLOSED`.
    pub status: String,
    #[serde(default)]
    pub accounts: Vec<Sensitive<String>>,
    pub access: Access,
}

/// Decimal string amount as sent by the API (`"-?\d+(\.\d+)?"`); parse with
/// [`crate::money::parse_amount`], never as a float.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Amount {
    pub currency: String,
    pub amount: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Balance {
    #[serde(default)]
    pub name: Option<Sensitive<String>>,
    pub balance_amount: Sensitive<Amount>,
    pub balance_type: String,
    #[serde(default)]
    pub reference_date: Option<NaiveDate>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Party {
    #[serde(default)]
    pub name: Option<Sensitive<String>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum CreditDebitIndicator {
    #[serde(rename = "CRDT")]
    Crdt,
    #[serde(rename = "DBIT")]
    Dbit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum TransactionStatus {
    #[serde(rename = "BOOK")]
    Book,
    #[serde(rename = "CNCL")]
    Cncl,
    #[serde(rename = "HOLD")]
    Hold,
    #[serde(rename = "OTHR")]
    Othr,
    #[serde(rename = "PDNG")]
    Pdng,
    #[serde(rename = "RJCT")]
    Rjct,
    #[serde(rename = "SCHD")]
    Schd,
}

impl TransactionStatus {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Book => "BOOK",
            Self::Cncl => "CNCL",
            Self::Hold => "HOLD",
            Self::Othr => "OTHR",
            Self::Pdng => "PDNG",
            Self::Rjct => "RJCT",
            Self::Schd => "SCHD",
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Transaction {
    /// Unique and immutable per account across sessions, when provided.
    #[serde(default)]
    pub entry_reference: Option<Sensitive<String>>,
    /// Not unique, may change between fetches.
    #[serde(default)]
    pub transaction_id: Option<Sensitive<String>>,
    pub transaction_amount: Sensitive<Amount>,
    pub credit_debit_indicator: CreditDebitIndicator,
    pub status: TransactionStatus,
    #[serde(default)]
    pub booking_date: Option<NaiveDate>,
    #[serde(default)]
    pub value_date: Option<NaiveDate>,
    #[serde(default)]
    pub transaction_date: Option<NaiveDate>,
    #[serde(default)]
    pub creditor: Option<Sensitive<Party>>,
    #[serde(default)]
    pub debtor: Option<Sensitive<Party>>,
    #[serde(default, deserialize_with = "null_as_empty")]
    pub remittance_information: Sensitive<Vec<String>>,
    #[serde(default)]
    pub note: Option<Sensitive<String>>,
    #[serde(default)]
    pub balance_after_transaction: Option<Sensitive<Amount>>,
}

fn null_as_empty<'de, D>(d: D) -> Result<Sensitive<Vec<String>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Ok(Sensitive::new(Option::<Vec<String>>::deserialize(d)?.unwrap_or_default()))
}

impl<T: Default> Default for Sensitive<T> {
    fn default() -> Self {
        Self(T::default())
    }
}

// Wire wrappers (crate-private).

#[derive(Deserialize)]
pub(crate) struct AspspsPage {
    pub aspsps: Vec<Aspsp>,
}

#[derive(Deserialize)]
pub(crate) struct BalancesPage {
    pub balances: Vec<Balance>,
}

#[derive(Deserialize)]
pub(crate) struct TransactionsPage {
    pub transactions: Vec<Transaction>,
    #[serde(default)]
    pub continuation_key: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct ErrorBody {
    #[serde(default)]
    pub error: Option<String>,
}
