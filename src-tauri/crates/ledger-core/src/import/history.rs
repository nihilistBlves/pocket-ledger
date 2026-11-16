//! Import history and undo.
//!
//! Undo deletes exactly the transactions a batch inserted (`import_batch_id = batch`
//! and `source = 'import'`). Rows that already existed, or that the batch saw as
//! duplicates, were never tagged with the batch, so they are untouched by
//! construction. The batch row stays as history with `undone_at` set.
//!
//! Signal for "adopted by bank sync": when bank sync adopts a reference onto, or
//! rewrites, a row whose `source` is `import`, `dedup::apply` stamps the row with the
//! sync run id (`sync_run_id`). Imported rows never have one otherwise, so a batch
//! row with `sync_run_id IS NOT NULL` means the bank now owns it and the batch can not
//! be undone. Rows adopted before this signal existed carry no stamp (not detectable).

use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;

use super::ImportError;
use crate::rules;

/// Largest `limit` accepted by [`list_import_batches`].
pub const MAX_HISTORY_LIMIT: usize = 200;

/// One imported file, for the history list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportBatchRow {
    pub id: i64,
    pub account_id: i64,
    pub account_name: String,
    pub profile: String,
    /// File name only (never a directory), as it was when imported.
    pub file_name: Option<String>,
    /// UTC ISO timestamp.
    pub imported_at: String,
    pub rows_total: i64,
    pub inserted: i64,
    pub updated: i64,
    pub duplicates: i64,
    pub errors: i64,
    pub undone_at: Option<String>,
    /// Not undone, inserted something, and no row was adopted by bank sync.
    pub can_undo: bool,
}

/// The most recent batches, newest first.
pub fn list_import_batches(
    conn: &Connection,
    limit: usize,
) -> Result<Vec<ImportBatchRow>, ImportError> {
    let limit = i64::try_from(limit.clamp(1, MAX_HISTORY_LIMIT)).unwrap_or(1);
    let mut stmt = conn.prepare(
        "SELECT b.id, b.account_id, a.name, b.profile, b.file_name, b.imported_at,
                b.rows_total, b.inserted, b.updated, b.duplicates, b.errors, b.undone_at,
                (b.undone_at IS NULL AND b.inserted > 0 AND NOT EXISTS (
                    SELECT 1 FROM transactions t
                    WHERE t.import_batch_id = b.id AND t.source = 'import'
                      AND t.sync_run_id IS NOT NULL))
         FROM import_batches b JOIN accounts a ON a.id = b.account_id
         ORDER BY b.imported_at DESC, b.id DESC
         LIMIT ?1",
    )?;
    let rows = stmt.query_map([limit], |r| {
        Ok(ImportBatchRow {
            id: r.get(0)?,
            account_id: r.get(1)?,
            account_name: r.get(2)?,
            profile: r.get(3)?,
            file_name: r.get(4)?,
            imported_at: r.get(5)?,
            rows_total: r.get(6)?,
            inserted: r.get(7)?,
            updated: r.get(8)?,
            duplicates: r.get(9)?,
            errors: r.get(10)?,
            undone_at: r.get(11)?,
            can_undo: r.get::<_, i64>(12)? == 1,
        })
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// What an undo did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UndoReport {
    /// Transactions deleted.
    pub deleted: usize,
    /// Surviving rows that were linked as an internal transfer with a deleted one and
    /// are now unlinked (and uncategorized, then run through the rules again).
    pub transfers_unlinked: usize,
}

/// Deletes the transactions `batch_id` inserted and marks the batch undone, in one
/// transaction. Nothing is written when it fails. Refused when the batch is unknown
/// ([`ImportError::BatchNotFound`]), already undone ([`ImportError::BatchAlreadyUndone`])
/// or holds rows adopted by bank sync ([`ImportError::BatchHasSyncedRows`]).
pub fn undo_import_batch(conn: &mut Connection, batch_id: i64) -> Result<UndoReport, ImportError> {
    let tx = conn.transaction()?;
    let undone: Option<Option<String>> = tx
        .query_row(
            "SELECT undone_at FROM import_batches WHERE id = ?1",
            [batch_id],
            |r| r.get(0),
        )
        .optional()?;
    match undone {
        None => return Err(ImportError::BatchNotFound),
        Some(Some(_)) => return Err(ImportError::BatchAlreadyUndone),
        Some(None) => {}
    }

    // (id, peer, synced)
    let rows: Vec<(i64, Option<i64>, bool)> = {
        let mut stmt = tx.prepare(
            "SELECT id, transfer_peer_id, sync_run_id IS NOT NULL FROM transactions
             WHERE import_batch_id = ?1 AND source = 'import'",
        )?;
        let mapped = stmt.query_map([batch_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
        mapped.collect::<Result<_, _>>()?
    };
    if rows.iter().any(|(_, _, synced)| *synced) {
        return Err(ImportError::BatchHasSyncedRows);
    }

    // Unlink surviving peers first, as `transfers::unlink_transfer` does: only a
    // category that came from the link is cleared, then the rules run again.
    let doomed: std::collections::HashSet<i64> = rows.iter().map(|(id, _, _)| *id).collect();
    let mut peers: Vec<i64> = rows
        .iter()
        .filter_map(|(_, peer, _)| *peer)
        .filter(|peer| !doomed.contains(peer))
        .collect();
    peers.sort_unstable();
    peers.dedup();
    for peer in &peers {
        tx.execute(
            "UPDATE transactions
             SET transfer_peer_id = NULL,
                 category_id = CASE WHEN category_source = 'transfer' THEN NULL ELSE category_id END,
                 category_source = CASE WHEN category_source = 'transfer' THEN 'none' ELSE category_source END,
                 rule_id = CASE WHEN category_source = 'transfer' THEN NULL ELSE rule_id END
             WHERE id = ?1",
            [peer],
        )?;
    }
    rules::categorize(&tx, &peers)?;

    // Tags and the other rows' peer links go with the foreign keys
    // (ON DELETE CASCADE / SET NULL).
    let mut deleted = 0;
    for (id, _, _) in &rows {
        deleted += tx.execute("DELETE FROM transactions WHERE id = ?1", [id])?;
    }
    tx.execute(
        "UPDATE import_batches SET undone_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ?1",
        params![batch_id],
    )?;
    tx.commit()?;
    Ok(UndoReport {
        deleted,
        transfers_unlinked: peers.len(),
    })
}
