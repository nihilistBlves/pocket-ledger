//! Mutations of the transaction list. Each one runs in a SQLite transaction and
//! returns the rows as the list would show them afterwards.

use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;

use super::transactions::rows_by_ids;
use super::{LedgerError, Result, TxRow, MAX_NOTES_CHARS, MAX_TAGS, MAX_TAG_CHARS};
use crate::normalize::merchant_key;
use crate::rules::{self, NewRule};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SetCategoryResult {
    pub row: TxRow,
    /// Other transactions that took the category because of the rule(s) just stored.
    pub recategorized: usize,
    /// The manual rule that was created or repointed; `None` unless `create_rule`.
    pub rule_id: Option<i64>,
}

fn require_tx(conn: &Connection, tx_id: i64) -> Result<()> {
    let found: Option<i64> =
        conn.query_row("SELECT id FROM transactions WHERE id = ?1", [tx_id], |r| r.get(0)).optional()?;
    found.map(|_| ()).ok_or(LedgerError::NotFound("transaction"))
}

fn one_row(conn: &Connection, tx_id: i64) -> Result<TxRow> {
    rows_by_ids(conn, &[tx_id])?.into_iter().next().ok_or(LedgerError::NotFound("transaction"))
}

/// Sets the category by hand, learns a rule from it (similar rows follow) and, when
/// `create_rule` is set, also stores a user-owned "contains" rule on the merchant key.
pub fn set_category(conn: &Connection, tx_id: i64, category_id: i64, create_rule: bool) -> Result<SetCategoryResult> {
    let dbtx = conn.unchecked_transaction()?;
    require_tx(&dbtx, tx_id)?;
    let category_exists: Option<i64> = dbtx
        .query_row("SELECT id FROM categories WHERE id = ?1", [category_id], |r| r.get(0))
        .optional()?;
    if category_exists.is_none() {
        return Err(LedgerError::NotFound("category"));
    }

    let learned = rules::learn_from_correction(&dbtx, tx_id, category_id)?;
    let mut recategorized = learned.recategorized;
    let mut rule_id = None;

    if create_rule {
        let (description, counterparty): (String, Option<String>) = dbtx.query_row(
            "SELECT description, counterparty FROM transactions WHERE id = ?1",
            [tx_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        if let Some(key) = merchant_key(&description, counterparty.as_deref()) {
            let existing: Option<i64> = dbtx
                .query_row(
                    "SELECT id FROM rules WHERE pattern = ?1 AND field = 'any' AND match_type = 'contains'
                       AND account_id IS NULL AND direction = 'any'",
                    [&key],
                    |r| r.get(0),
                )
                .optional()?;
            let id = match existing {
                Some(id) => {
                    dbtx.execute("UPDATE rules SET category_id = ?1, origin = 'manual' WHERE id = ?2", params![category_id, id])?;
                    id
                }
                None => rules::create_manual_rule(&dbtx, &NewRule::contains(&key, category_id))?,
            };
            rule_id = Some(id);
            // Rows that are free to follow rules; the broad manual rule may match more
            // than the learned (same-merchant) one, so every such row is re-run.
            let candidates = {
                let mut stmt = dbtx.prepare(
                    "SELECT id FROM transactions
                     WHERE id != ?1 AND category_source IN ('none', 'rule', 'learned')",
                )?;
                let ids = stmt.query_map([tx_id], |r| r.get::<_, i64>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
                ids
            };
            rules::categorize(&dbtx, &candidates)?;
            // Rows now following the manual rule (the learned rule may have moved some already).
            let following: i64 = dbtx.query_row(
                "SELECT count(*) FROM transactions WHERE rule_id = ?1 AND id != ?2",
                params![id, tx_id],
                |r| r.get(0),
            )?;
            let following = usize::try_from(following).unwrap_or(0);
            recategorized = recategorized.max(following);
        }
    }

    let row = one_row(&dbtx, tx_id)?;
    dbtx.commit()?;
    Ok(SetCategoryResult { row, recategorized, rule_id })
}

/// Trimmed notes; empty becomes `None`.
pub fn set_notes(conn: &Connection, tx_id: i64, notes: Option<&str>) -> Result<TxRow> {
    let cleaned = notes.map(str::trim).filter(|n| !n.is_empty());
    if cleaned.is_some_and(|n| n.chars().count() > MAX_NOTES_CHARS) {
        return Err(LedgerError::InvalidInput("notes are too long"));
    }
    let dbtx = conn.unchecked_transaction()?;
    require_tx(&dbtx, tx_id)?;
    dbtx.execute("UPDATE transactions SET notes = ?1 WHERE id = ?2", params![cleaned, tx_id])?;
    let row = one_row(&dbtx, tx_id)?;
    dbtx.commit()?;
    Ok(row)
}

/// Trims, drops empties and de-duplicates case-insensitively (first spelling wins).
fn clean_tags(tags: &[String]) -> Result<Vec<String>> {
    let mut out: Vec<String> = Vec::new();
    for raw in tags {
        let tag = raw.trim();
        if tag.is_empty() {
            continue;
        }
        if tag.chars().count() > MAX_TAG_CHARS {
            return Err(LedgerError::InvalidInput("tag is too long"));
        }
        if tag.chars().any(char::is_control) {
            return Err(LedgerError::InvalidInput("tag has invalid characters"));
        }
        let lower = tag.to_lowercase();
        if !out.iter().any(|t| t.to_lowercase() == lower) {
            out.push(tag.to_string());
        }
    }
    if out.len() > MAX_TAGS {
        return Err(LedgerError::InvalidInput("too many tags"));
    }
    Ok(out)
}

/// Replaces the tags of a transaction. Tags left without rows are kept for suggestions.
pub fn set_tags(conn: &Connection, tx_id: i64, tags: &[String]) -> Result<TxRow> {
    let tags = clean_tags(tags)?;
    let dbtx = conn.unchecked_transaction()?;
    require_tx(&dbtx, tx_id)?;
    dbtx.execute("DELETE FROM transaction_tags WHERE transaction_id = ?1", [tx_id])?;
    for tag in &tags {
        // `tags.name` is NOCASE-unique, so an existing spelling is reused.
        dbtx.execute("INSERT OR IGNORE INTO tags (name) VALUES (?1)", [tag])?;
        let tag_id: i64 = dbtx.query_row("SELECT id FROM tags WHERE name = ?1", [tag], |r| r.get(0))?;
        dbtx.execute(
            "INSERT OR IGNORE INTO transaction_tags (transaction_id, tag_id) VALUES (?1, ?2)",
            params![tx_id, tag_id],
        )?;
    }
    let row = one_row(&dbtx, tx_id)?;
    dbtx.commit()?;
    Ok(row)
}

/// Undoes an automatic transfer link (either side); returns both rows afterwards.
pub fn unlink_transfer(conn: &Connection, tx_id: i64) -> Result<Vec<TxRow>> {
    let dbtx = conn.unchecked_transaction()?;
    require_tx(&dbtx, tx_id)?;
    let ids = crate::transfers::unlink_transfer(&dbtx, tx_id)?;
    let rows = rows_by_ids(&dbtx, &ids)?;
    dbtx.commit()?;
    Ok(rows)
}
