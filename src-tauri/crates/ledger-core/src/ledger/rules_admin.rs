//! Management of categorization rules behind the "Reglas" screen
//! (docs/decisions/phase-7.md): ordered list, save (with optional re-run on the last
//! 90 days), delete, dry-run test and "apply to uncategorized".
//!
//! Matching and evaluation order are those of [`crate::rules`]; this module only adds
//! validation, listing and the simulation of a rule that is not stored yet.

use chrono::Duration;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use super::period::{iso, parse_date};
use super::{LedgerError, Result, EFFECTIVE_DATE};
use crate::normalize::normalize_text;
use crate::rules::{categorize, load_rules, rule_matches, StoredRule, TxView};

pub const MAX_PATTERN_CHARS: usize = 100;
/// Length of the window (in days, `today` included) used to test and apply a rule.
pub const WINDOW_DAYS: i64 = 90;
pub const MAX_TEST_ROWS: usize = 50;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuleInput {
    /// Normalized by the core; 1-100 chars after normalization.
    pub pattern: String,
    /// `any`, `description` or `counterparty`.
    pub field: String,
    /// `contains` or `equals`.
    pub match_type: String,
    pub account_id: Option<i64>,
    /// `any`, `in` or `out`.
    pub direction: String,
    pub category_id: i64,
    pub priority: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuleRow {
    pub id: i64,
    /// 1-based position in the evaluation order.
    pub order: i64,
    pub pattern: String,
    pub field: String,
    pub match_type: String,
    pub account_id: Option<i64>,
    pub direction: String,
    pub category_id: i64,
    pub priority: i64,
    /// `manual` or `learned`.
    pub origin: String,
    pub account_name: Option<String>,
    pub category_name: String,
    pub parent_category_name: Option<String>,
    pub hits: i64,
    pub last_hit_at: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveRuleResult {
    pub rule: RuleRow,
    pub recategorized: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyResult {
    pub recategorized: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuleTestRow {
    pub tx_id: i64,
    pub date: String,
    pub description: String,
    pub counterparty: Option<String>,
    pub amount_cents: i64,
    pub category_id: Option<i64>,
    pub category_name: Option<String>,
    /// `change`, `unchanged`, `manual` or `other_rule`.
    pub outcome: &'static str,
    pub winning_rule_id: Option<i64>,
    pub winning_rule_pattern: Option<String>,
    pub winning_rule_order: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuleTest {
    pub from: String,
    pub to: String,
    pub matched: i64,
    pub change: i64,
    pub unchanged: i64,
    pub manual: i64,
    pub other_rule: i64,
    pub rows: Vec<RuleTestRow>,
}

/// Checks a rule as typed by the user and returns it with the pattern normalized.
fn validate(conn: &Connection, input: &RuleInput) -> Result<RuleInput> {
    let pattern = normalize_text(&input.pattern);
    let len = pattern.chars().count();
    if len == 0 || len > MAX_PATTERN_CHARS {
        return Err(LedgerError::InvalidInput(
            "pattern must have 1-100 characters",
        ));
    }
    if !matches!(input.field.as_str(), "any" | "description" | "counterparty") {
        return Err(LedgerError::InvalidInput("unknown rule field"));
    }
    if !matches!(input.match_type.as_str(), "contains" | "equals") {
        return Err(LedgerError::InvalidInput("unknown match type"));
    }
    if !matches!(input.direction.as_str(), "any" | "in" | "out") {
        return Err(LedgerError::InvalidInput("unknown direction"));
    }
    if let Some(account) = input.account_id {
        let found: Option<i64> = conn
            .query_row("SELECT id FROM accounts WHERE id = ?1", [account], |r| {
                r.get(0)
            })
            .optional()?;
        if found.is_none() {
            return Err(LedgerError::InvalidInput("unknown account"));
        }
    }
    let category: Option<(String, i64)> = conn
        .query_row(
            "SELECT kind, is_system FROM categories WHERE id = ?1",
            [input.category_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let Some((kind, is_system)) = category else {
        return Err(LedgerError::NotFound("category"));
    };
    if is_system != 0 || !matches!(kind.as_str(), "income" | "expense") {
        return Err(LedgerError::InvalidInput(
            "rules cannot point to a transfer or system category",
        ));
    }
    Ok(RuleInput {
        pattern,
        ..input.clone()
    })
}

/// All rules in evaluation order: manual before learned, then priority desc, pattern
/// length desc, id desc.
pub fn list_rules(conn: &Connection) -> Result<Vec<RuleRow>> {
    let mut stmt = conn.prepare(
        "SELECT r.id, r.pattern, r.field, r.match_type, r.account_id, r.direction, r.category_id,
                r.priority, r.origin, a.name, c.name, pc.name, r.hits, r.last_hit_at, r.created_at
         FROM rules r
         LEFT JOIN accounts a ON a.id = r.account_id
         JOIN categories c ON c.id = r.category_id
         LEFT JOIN categories pc ON pc.id = c.parent_id
         ORDER BY (r.origin = 'manual') DESC, r.priority DESC, length(r.pattern) DESC, r.id DESC",
    )?;
    let mut rows = stmt
        .query_map([], |r| {
            Ok(RuleRow {
                id: r.get(0)?,
                order: 0,
                pattern: r.get(1)?,
                field: r.get(2)?,
                match_type: r.get(3)?,
                account_id: r.get(4)?,
                direction: r.get(5)?,
                category_id: r.get(6)?,
                priority: r.get(7)?,
                origin: r.get(8)?,
                account_name: r.get(9)?,
                category_name: r.get(10)?,
                parent_category_name: r.get(11)?,
                hits: r.get(12)?,
                last_hit_at: r.get(13)?,
                created_at: r.get(14)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for (i, row) in rows.iter_mut().enumerate() {
        row.order = i64::try_from(i).unwrap_or(i64::MAX) + 1;
    }
    Ok(rows)
}

/// The window of rows a rule is tested against and applied to: the 90 days up to `today`.
fn window(today: &str) -> Result<(String, String)> {
    let to = parse_date(today)?;
    let from = to - Duration::days(WINDOW_DAYS - 1);
    Ok((iso(from), iso(to)))
}

struct WindowRow {
    id: i64,
    date: String,
    description: String,
    counterparty: Option<String>,
    amount_cents: i64,
    account_id: i64,
    category_id: Option<i64>,
    category_name: Option<String>,
    source: String,
}

/// Own-account rows with an effective date in `[from, to]`, newest first.
fn window_rows(conn: &Connection, from: &str, to: &str) -> Result<Vec<WindowRow>> {
    let sql = format!(
        "SELECT t.id, {EFFECTIVE_DATE}, t.description, t.counterparty, t.amount_cents, t.account_id,
                t.category_id, c.name, t.category_source
         FROM transactions t
         JOIN accounts a ON a.id = t.account_id AND a.is_own = 1
         LEFT JOIN categories c ON c.id = t.category_id
         WHERE {EFFECTIVE_DATE} BETWEEN ?1 AND ?2
         ORDER BY {EFFECTIVE_DATE} DESC, t.id DESC"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map(params![from, to], |r| {
            Ok(WindowRow {
                id: r.get(0)?,
                date: r.get(1)?,
                description: r.get(2)?,
                counterparty: r.get(3)?,
                amount_cents: r.get(4)?,
                account_id: r.get(5)?,
                category_id: r.get(6)?,
                category_name: r.get(7)?,
                source: r.get(8)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

fn view_of(row: &WindowRow) -> TxView {
    TxView {
        account_id: row.account_id,
        amount_cents: row.amount_cents,
        category_source: row.source.clone(),
        counterparty: normalize_text(row.counterparty.as_deref().unwrap_or("")),
        description: normalize_text(&row.description),
    }
}

fn category_of(conn: &Connection, tx_id: i64) -> Result<Option<i64>> {
    Ok(conn.query_row(
        "SELECT category_id FROM transactions WHERE id = ?1",
        [tx_id],
        |r| r.get(0),
    )?)
}

/// Runs the rules over `ids` and returns how many rows ended up in a different category.
fn run_rules(conn: &Connection, ids: &[i64]) -> Result<i64> {
    let mut before = Vec::with_capacity(ids.len());
    for &id in ids {
        before.push(category_of(conn, id)?);
    }
    categorize(conn, ids)?;
    let mut changed = 0;
    for (&id, was) in ids.iter().zip(before) {
        if category_of(conn, id)? != was {
            changed += 1;
        }
    }
    Ok(changed)
}

fn require_rule(conn: &Connection, id: i64) -> Result<()> {
    let found: Option<i64> = conn
        .query_row("SELECT id FROM rules WHERE id = ?1", [id], |r| r.get(0))
        .optional()?;
    found.map(|_| ()).ok_or(LedgerError::NotFound("rule"))
}

/// Creates (`id` = `None`) or edits a rule. The saved rule is always `manual`: editing a
/// learned rule makes it the user's. With `apply_to_existing`, the rules are re-run on the
/// rows of the last 90 days that this rule matches and whose category is not manual.
pub fn save_rule(
    conn: &Connection,
    id: Option<i64>,
    input: &RuleInput,
    apply_to_existing: bool,
    today: &str,
) -> Result<SaveRuleResult> {
    let (from, to) = window(today)?;
    let dbtx = conn.unchecked_transaction()?;
    let rule = validate(&dbtx, input)?;
    if let Some(id) = id {
        require_rule(&dbtx, id)?;
    }
    let clash: Option<i64> = dbtx
        .query_row(
            "SELECT id FROM rules
             WHERE pattern = ?1 AND field = ?2 AND match_type = ?3
               AND COALESCE(account_id, 0) = COALESCE(?4, 0) AND direction = ?5
               AND (?6 IS NULL OR id != ?6)",
            params![
                rule.pattern,
                rule.field,
                rule.match_type,
                rule.account_id,
                rule.direction,
                id
            ],
            |r| r.get(0),
        )
        .optional()?;
    if clash.is_some() {
        return Err(LedgerError::Conflict("an identical rule already exists"));
    }
    let rule_id = match id {
        Some(id) => {
            dbtx.execute(
                "UPDATE rules SET pattern = ?1, field = ?2, match_type = ?3, account_id = ?4, direction = ?5,
                        category_id = ?6, priority = ?7, origin = 'manual',
                        updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                 WHERE id = ?8",
                params![
                    rule.pattern,
                    rule.field,
                    rule.match_type,
                    rule.account_id,
                    rule.direction,
                    rule.category_id,
                    rule.priority,
                    id
                ],
            )?;
            id
        }
        None => {
            dbtx.execute(
                "INSERT INTO rules (pattern, field, match_type, account_id, direction, category_id, origin, priority)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'manual', ?7)",
                params![
                    rule.pattern,
                    rule.field,
                    rule.match_type,
                    rule.account_id,
                    rule.direction,
                    rule.category_id,
                    rule.priority
                ],
            )?;
            dbtx.last_insert_rowid()
        }
    };

    let mut recategorized = 0;
    if apply_to_existing {
        let saved = load_rules(&dbtx)?
            .into_iter()
            .find(|r| r.id == rule_id)
            .ok_or(LedgerError::NotFound("rule"))?;
        let candidates: Vec<i64> = window_rows(&dbtx, &from, &to)?
            .iter()
            .filter(|row| matches!(row.source.as_str(), "none" | "rule" | "learned"))
            .filter(|row| rule_matches(&saved, &view_of(row)))
            .map(|row| row.id)
            .collect();
        recategorized = run_rules(&dbtx, &candidates)?;
    }

    let row = list_rules(&dbtx)?
        .into_iter()
        .find(|r| r.id == rule_id)
        .ok_or(LedgerError::NotFound("rule"))?;
    dbtx.commit()?;
    Ok(SaveRuleResult {
        rule: row,
        recategorized,
    })
}

/// Deletes a rule. Rows it categorized keep their category (`rule_id` becomes NULL).
pub fn delete_rule(conn: &Connection, id: i64) -> Result<()> {
    let n = conn.execute("DELETE FROM rules WHERE id = ?1", [id])?;
    if n == 0 {
        return Err(LedgerError::NotFound("rule"));
    }
    Ok(())
}

/// Re-runs every rule over own-account rows without a category (`category_source = 'none'`
/// or in "Sin categoría"), all history. Manual and transfer rows are never touched.
pub fn apply_rules_uncategorized(conn: &Connection) -> Result<ApplyResult> {
    let dbtx = conn.unchecked_transaction()?;
    let ids: Vec<i64> = {
        let mut stmt = dbtx.prepare(
            "SELECT t.id FROM transactions t
             JOIN accounts a ON a.id = t.account_id AND a.is_own = 1
             LEFT JOIN categories c ON c.id = t.category_id
             WHERE t.category_source = 'none' OR c.key = 'uncategorized'
             ORDER BY t.id",
        )?;
        let ids = stmt
            .query_map([], |r| r.get(0))?
            .collect::<rusqlite::Result<Vec<i64>>>()?;
        ids
    };
    let recategorized = run_rules(&dbtx, &ids)?;
    dbtx.commit()?;
    Ok(ApplyResult { recategorized })
}

/// Evaluation order as the list shows it, including the simulated rule in its place.
fn sort_rules(rules: &mut [StoredRule]) {
    rules.sort_by(|a, b| {
        (b.origin == "manual")
            .cmp(&(a.origin == "manual"))
            .then(b.priority.cmp(&a.priority))
            .then(b.pattern.chars().count().cmp(&a.pattern.chars().count()))
            .then(b.id.cmp(&a.id))
    });
}

fn rank(outcome: &str) -> u8 {
    match outcome {
        "change" => 0,
        "other_rule" => 1,
        "manual" => 2,
        _ => 3,
    }
}

/// Dry run: how the rule, as it would be saved (manual, new values), would affect the
/// own-account rows of the last 90 days. Nothing is written.
pub fn test_rule(
    conn: &Connection,
    id: Option<i64>,
    input: &RuleInput,
    today: &str,
) -> Result<RuleTest> {
    let (from, to) = window(today)?;
    let rule = validate(conn, input)?;
    if let Some(id) = id {
        require_rule(conn, id)?;
    }
    let sim_id = id.unwrap_or(i64::MAX);
    let mut rules: Vec<StoredRule> = load_rules(conn)?
        .into_iter()
        .filter(|r| Some(r.id) != id)
        .collect();
    rules.push(StoredRule {
        id: sim_id,
        pattern: rule.pattern,
        field: rule.field,
        match_type: rule.match_type,
        account_id: rule.account_id,
        direction: rule.direction,
        category_id: rule.category_id,
        origin: "manual".to_string(),
        priority: rule.priority,
    });
    sort_rules(&mut rules);

    let mut test = RuleTest {
        from: from.clone(),
        to: to.clone(),
        matched: 0,
        change: 0,
        unchanged: 0,
        manual: 0,
        other_rule: 0,
        rows: Vec::new(),
    };
    let mut rows = Vec::new();
    for row in window_rows(conn, &from, &to)? {
        let view = view_of(&row);
        let Some(winner) = rules.iter().position(|r| rule_matches(r, &view)) else {
            continue;
        };
        let simulated_matches = rules
            .iter()
            .find(|r| r.id == sim_id)
            .is_some_and(|r| rule_matches(r, &view));
        if !simulated_matches {
            continue;
        }
        test.matched += 1;
        let (outcome, win) = if matches!(row.source.as_str(), "manual" | "transfer") {
            test.manual += 1;
            ("manual", None)
        } else if rules[winner].id == sim_id {
            if row.category_id == Some(rules[winner].category_id) {
                test.unchanged += 1;
                ("unchanged", None)
            } else {
                test.change += 1;
                ("change", None)
            }
        } else {
            test.other_rule += 1;
            ("other_rule", Some(winner))
        };
        rows.push(RuleTestRow {
            tx_id: row.id,
            date: row.date,
            description: row.description,
            counterparty: row.counterparty,
            amount_cents: row.amount_cents,
            category_id: row.category_id,
            category_name: row.category_name,
            outcome,
            winning_rule_id: win.map(|w| rules[w].id),
            winning_rule_pattern: win.map(|w| rules[w].pattern.clone()),
            winning_rule_order: win.map(|w| i64::try_from(w).unwrap_or(i64::MAX) + 1),
        });
    }
    // Stable sort: rows are already newest first.
    rows.sort_by_key(|r| rank(r.outcome));
    rows.truncate(MAX_TEST_ROWS);
    test.rows = rows;
    Ok(test)
}
