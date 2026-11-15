//! Source-independent transaction shape fed into the ingest pipeline.
//!
//! Bank sync (Enable Banking) and file imports (CSV/Excel) both convert their
//! rows into [`NewTransaction`] and hand a batch to `ingest::ingest`, so dedup,
//! internal-transfer detection and categorization behave the same for every
//! source. Amounts are already signed cents (negative = outflow).

/// Lifecycle state reported by the source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TxStatus {
    Pending,
    Booked,
}

impl TxStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            TxStatus::Pending => "pending",
            TxStatus::Booked => "booked",
        }
    }
}

/// Where a transaction came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TxSource {
    Bank,
    Import,
    Manual,
}

impl TxSource {
    pub fn as_str(self) -> &'static str {
        match self {
            TxSource::Bank => "bank",
            TxSource::Import => "import",
            TxSource::Manual => "manual",
        }
    }
}

/// One normalized transaction, before dedup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewTransaction {
    pub status: TxStatus,
    /// `YYYY-MM-DD`. Required when `status` is `Booked`.
    pub booking_date: Option<String>,
    /// `YYYY-MM-DD`.
    pub value_date: Option<String>,
    /// Signed cents: negative = money leaving the account.
    pub amount_cents: i64,
    /// ISO 4217, e.g. `EUR`.
    pub currency: String,
    /// Main human-readable text (bank "concepto").
    pub description: String,
    /// Creditor (for outflows) or debtor (for inflows) name, if known.
    pub counterparty: Option<String>,
    /// Remittance information joined into one string, if any.
    pub remittance: Option<String>,
    /// Bank-provided stable reference (`entry_reference`), if any.
    /// Never the Enable Banking `transaction_id`, which is not stable.
    pub entry_reference: Option<String>,
}
