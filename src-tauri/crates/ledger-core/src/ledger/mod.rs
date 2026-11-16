//! Read models and edits behind the "Resumen" and "Movimientos" screens
//! (docs/decisions/phase-6.md).
//!
//! Accounting rules shared by every query here:
//! - effective date = `COALESCE(booking_date, value_date, date(created_at))`;
//! - pending rows are included;
//! - rows whose category has `kind = 'transfer'` are internal transfers and are
//!   excluded from income, expense and category totals;
//! - income is the sum of positive amounts, expense the absolute sum of negative
//!   ones (the sign decides, not the category kind);
//! - only the user's own accounts (`is_own = 1`) take part.
//!
//! Dates and "today" always come from the caller: nothing in here reads a clock.
//! Money is integer cents.
//!
//! Performance note: the effective-date expression cannot use `ix_transactions_booking`
//! directly, so date filters scan the (own-account) rows. That is fine at the data size
//! of one person's ledger (tens of thousands of rows); revisit with a generated column
//! if it ever shows up in a profile.

use serde::Serialize;
use thiserror::Error;

pub mod accounts;
pub mod budgets;
pub mod categories_admin;
pub mod dashboard;
pub mod edit;
pub mod period;
pub mod rules_admin;
pub mod transactions;

#[cfg(test)]
mod admin_tests;
#[cfg(test)]
mod tests;

pub use accounts::{create_manual_account, MAX_ACCOUNT_NAME_CHARS};
pub use budgets::{budgets, remove_budget, set_budget, BudgetMonth};
pub use categories_admin::{
    category_tree, create_category, delete_category, update_category, CategoryNode,
    DeleteCategoryResult,
};

pub use dashboard::{dashboard, Dashboard};
pub use edit::{set_category, set_notes, set_tags, unlink_transfer, SetCategoryResult};
pub use rules_admin::{
    apply_rules_uncategorized, delete_rule, list_rules, save_rule, test_rule, ApplyResult,
    RuleInput, RuleRow, RuleTest, SaveRuleResult,
};
pub use transactions::{categories, list_transactions, tag_names, CategoryOption, TxFilter, TxPage};

/// SQL expression for the effective date of a row aliased `t`.
pub(crate) const EFFECTIVE_DATE: &str =
    "COALESCE(t.booking_date, t.value_date, substr(t.created_at, 1, 10))";

pub const DEFAULT_LIMIT: i64 = 100;
pub const MAX_LIMIT: i64 = 500;
pub const MAX_NOTES_CHARS: usize = 2000;
pub const MAX_TAGS: usize = 20;
pub const MAX_TAG_CHARS: usize = 40;
pub const MAX_TEXT_CHARS: usize = 200;

/// Why a request was refused. Display texts are for logs only; the UI gets a fixed
/// message chosen by the variant.
#[derive(Debug, Error)]
pub enum LedgerError {
    #[error("invalid input: {0}")]
    InvalidInput(&'static str),
    #[error("not found: {0}")]
    NotFound(&'static str),
    /// Valid request that clashes with existing data (duplicate name or rule, budget
    /// overlap, delete blocked).
    #[error("conflict: {0}")]
    Conflict(&'static str),
    #[error("database error")]
    Db(#[from] rusqlite::Error),
}

pub type Result<T> = std::result::Result<T, LedgerError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PeriodTotals {
    pub income_cents: i64,
    pub expense_cents: i64,
    pub balance_cents: i64,
}

impl PeriodTotals {
    pub(crate) fn new(income_cents: i64, expense_cents: i64) -> Self {
        Self { income_cents, expense_cents, balance_cents: income_cents - expense_cents }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TxRow {
    pub id: i64,
    pub account_id: i64,
    pub account_name: String,
    /// `pending` or `booked`.
    pub status: String,
    /// Effective date.
    pub date: String,
    pub booking_date: Option<String>,
    pub value_date: Option<String>,
    pub amount_cents: i64,
    pub currency: String,
    pub description: String,
    pub counterparty: Option<String>,
    pub remittance: Option<String>,
    pub category_id: Option<i64>,
    pub category_name: Option<String>,
    pub category_key: Option<String>,
    pub parent_category_id: Option<i64>,
    pub parent_category_name: Option<String>,
    /// `none`, `rule`, `learned`, `manual` or `transfer`.
    pub category_source: String,
    pub is_transfer: bool,
    pub transfer_peer_id: Option<i64>,
    pub transfer_peer_account_name: Option<String>,
    pub notes: Option<String>,
    pub tags: Vec<String>,
}
