//! Management of the category tree behind the "Categorías" screen
//! (docs/decisions/phase-7.md): tree with month aggregates, create, update, delete.

use std::collections::{HashMap, HashSet};

use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;

use super::period::{iso, last_day_of_month, month_key, parse_month};
use super::{LedgerError, Result, EFFECTIVE_DATE};

pub const MAX_NAME_CHARS: usize = 60;
/// Colour tokens of the DESIGN.md chart palette.
pub const COLOR_TOKENS: [&str; 9] = [
    "cat-1", "cat-2", "cat-3", "cat-4", "cat-5", "cat-6", "cat-7", "cat-8", "other",
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CategoryNode {
    pub id: i64,
    pub parent_id: Option<i64>,
    pub key: Option<String>,
    pub name: String,
    /// `income`, `expense`, `transfer` or `system`.
    pub kind: String,
    pub is_system: bool,
    pub sort_order: i64,
    /// Stored colour token; `None` = default / inherit.
    pub color: Option<String>,
    pub child_count: i64,
    /// Own-account rows in the month (groups include their children).
    pub month_tx_count: i64,
    /// Signed net sum in the month; for `kind = 'transfer'` the absolute sum of outflows.
    pub month_amount_cents: i64,
    /// All history, all accounts (groups include children): what a delete would move.
    pub total_tx_count: i64,
    /// Rules pointing to the category (groups include children).
    pub rule_count: i64,
    /// Active budget in the month: `own`, `group` (on the parent) or `children`.
    pub budget: Option<&'static str>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteCategoryResult {
    pub moved_transactions: i64,
    pub repointed_rules: i64,
    pub deleted_rules: i64,
    pub deleted_budgets: i64,
}

#[derive(Default, Clone, Copy)]
struct Stats {
    month_count: i64,
    month_net: i64,
    month_outflow: i64,
    total_count: i64,
    rules: i64,
}

impl Stats {
    fn add(&mut self, o: &Stats) {
        self.month_count += o.month_count;
        self.month_net += o.month_net;
        self.month_outflow += o.month_outflow;
        self.total_count += o.total_count;
        self.rules += o.rules;
    }
}

struct Raw {
    id: i64,
    parent_id: Option<i64>,
    key: Option<String>,
    name: String,
    kind: String,
    is_system: bool,
    sort_order: i64,
    color: Option<String>,
}

fn id_map(
    conn: &Connection,
    sql: &str,
    args: impl rusqlite::Params,
) -> Result<HashMap<i64, Vec<i64>>> {
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt
        .query_map(args, |r| {
            let mut v = Vec::new();
            for i in 1..r.as_ref().column_count() {
                v.push(r.get::<_, i64>(i)?);
            }
            Ok((r.get::<_, i64>(0)?, v))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows.into_iter().collect())
}

/// Builds the tree. With `month` (`YYYY-MM`) the month figures and the budget flag are
/// filled in; with `None` they are zero / `None`.
fn build_tree(conn: &Connection, month: Option<&str>) -> Result<Vec<CategoryNode>> {
    let mut stmt = conn.prepare(
        "SELECT id, parent_id, key, name, kind, is_system, sort_order, color FROM categories
         ORDER BY COALESCE((SELECT p.sort_order FROM categories p WHERE p.id = categories.parent_id), sort_order),
                  COALESCE(parent_id, id), parent_id IS NOT NULL, sort_order, id",
    )?;
    let raws = stmt
        .query_map([], |r| {
            Ok(Raw {
                id: r.get(0)?,
                parent_id: r.get(1)?,
                key: r.get(2)?,
                name: r.get(3)?,
                kind: r.get(4)?,
                is_system: r.get::<_, i64>(5)? != 0,
                sort_order: r.get(6)?,
                color: r.get(7)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    drop(stmt);

    let mut own: HashMap<i64, Stats> = HashMap::new();
    for (id, v) in id_map(conn, "SELECT category_id, count(*) FROM transactions WHERE category_id IS NOT NULL GROUP BY category_id", [])? {
        own.entry(id).or_default().total_count = v[0];
    }
    for (id, v) in id_map(
        conn,
        "SELECT category_id, count(*) FROM rules GROUP BY category_id",
        [],
    )? {
        own.entry(id).or_default().rules = v[0];
    }
    let mut active: HashSet<i64> = HashSet::new();
    if let Some(m) = month {
        let first = parse_month(m)?;
        let (from, to) = (iso(first), iso(last_day_of_month(first)));
        let sql = format!(
            "SELECT t.category_id, count(*), COALESCE(sum(t.amount_cents), 0),
                    COALESCE(sum(CASE WHEN t.amount_cents < 0 THEN -t.amount_cents ELSE 0 END), 0)
             FROM transactions t JOIN accounts a ON a.id = t.account_id AND a.is_own = 1
             WHERE t.category_id IS NOT NULL AND {EFFECTIVE_DATE} BETWEEN ?1 AND ?2
             GROUP BY t.category_id"
        );
        for (id, v) in id_map(conn, &sql, params![from, to])? {
            let s = own.entry(id).or_default();
            s.month_count = v[0];
            s.month_net = v[1];
            s.month_outflow = v[2];
        }
        let key = month_key(first);
        for (id, _) in id_map(
            conn,
            "SELECT category_id, 0 FROM budgets WHERE start_month <= ?1 AND (end_month IS NULL OR end_month >= ?1)",
            [key],
        )? {
            active.insert(id);
        }
    }

    let mut children: HashMap<i64, Vec<i64>> = HashMap::new();
    for r in &raws {
        if let Some(p) = r.parent_id {
            children.entry(p).or_default().push(r.id);
        }
    }
    let nodes = raws
        .into_iter()
        .map(|r| {
            let kids = children.get(&r.id).map(Vec::as_slice).unwrap_or_default();
            let mut stats = own.get(&r.id).copied().unwrap_or_default();
            for k in kids {
                stats.add(&own.get(k).copied().unwrap_or_default());
            }
            let budget = if active.contains(&r.id) {
                Some("own")
            } else if r.parent_id.is_some_and(|p| active.contains(&p)) {
                Some("group")
            } else if kids.iter().any(|k| active.contains(k)) {
                Some("children")
            } else {
                None
            };
            CategoryNode {
                id: r.id,
                parent_id: r.parent_id,
                key: r.key,
                month_amount_cents: if r.kind == "transfer" {
                    stats.month_outflow
                } else {
                    stats.month_net
                },
                kind: r.kind,
                name: r.name,
                is_system: r.is_system,
                sort_order: r.sort_order,
                color: r.color,
                child_count: i64::try_from(kids.len()).unwrap_or(i64::MAX),
                month_tx_count: stats.month_count,
                total_tx_count: stats.total_count,
                rule_count: stats.rules,
                budget,
            }
        })
        .collect();
    Ok(nodes)
}

/// Groups in `sort_order`, each followed by its subcategories, with the figures of `month`.
pub fn category_tree(conn: &Connection, month: &str) -> Result<Vec<CategoryNode>> {
    parse_month(month)?;
    build_tree(conn, Some(month))
}

/// One node without month figures (used as the result of create / update).
fn node(conn: &Connection, id: i64) -> Result<CategoryNode> {
    build_tree(conn, None)?
        .into_iter()
        .find(|n| n.id == id)
        .ok_or(LedgerError::NotFound("category"))
}

fn clean_name(name: &str) -> Result<String> {
    let name = name.trim();
    let len = name.chars().count();
    if len == 0 || len > MAX_NAME_CHARS {
        return Err(LedgerError::InvalidInput("name must have 1-60 characters"));
    }
    if name.chars().any(char::is_control) {
        return Err(LedgerError::InvalidInput("name has invalid characters"));
    }
    Ok(name.to_string())
}

fn check_color(color: Option<&str>) -> Result<()> {
    match color {
        Some(c) if !COLOR_TOKENS.contains(&c) => Err(LedgerError::InvalidInput("unknown colour")),
        _ => Ok(()),
    }
}

struct Cat {
    parent_id: Option<i64>,
    name: String,
    kind: String,
    is_system: bool,
}

fn load(conn: &Connection, id: i64) -> Result<Cat> {
    conn.query_row(
        "SELECT parent_id, name, kind, is_system FROM categories WHERE id = ?1",
        [id],
        |r| {
            Ok(Cat {
                parent_id: r.get(0)?,
                name: r.get(1)?,
                kind: r.get(2)?,
                is_system: r.get::<_, i64>(3)? != 0,
            })
        },
    )
    .optional()?
    .ok_or(LedgerError::NotFound("category"))
}

/// Case-insensitive duplicate check among the children of `parent_id` (`None` = groups).
fn name_taken(
    conn: &Connection,
    parent_id: Option<i64>,
    name: &str,
    except: Option<i64>,
) -> Result<bool> {
    let lower = name.to_lowercase();
    let mut stmt = conn.prepare(
        "SELECT id, name FROM categories WHERE COALESCE(parent_id, 0) = COALESCE(?1, 0)",
    )?;
    let rows = stmt
        .query_map([parent_id], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows
        .iter()
        .any(|(id, n)| Some(*id) != except && n.to_lowercase() == lower))
}

/// A parent for a subcategory: an existing, non-system group.
fn require_group(conn: &Connection, id: i64) -> Result<Cat> {
    let parent = load(conn, id)?;
    if parent.parent_id.is_some() || parent.is_system {
        return Err(LedgerError::InvalidInput(
            "parent must be a non-system group",
        ));
    }
    Ok(parent)
}

/// New group (`parent_id = None`, `kind` required: `income` or `expense`) or new
/// subcategory (kind inherited, `kind` ignored). Groups go last in the order.
pub fn create_category(
    conn: &Connection,
    parent_id: Option<i64>,
    name: &str,
    kind: Option<&str>,
) -> Result<CategoryNode> {
    let name = clean_name(name)?;
    let dbtx = conn.unchecked_transaction()?;
    let (kind, sort_order) = match parent_id {
        Some(pid) => {
            let parent = require_group(&dbtx, pid)?;
            let next: i64 = dbtx.query_row(
                "SELECT COALESCE(MAX(sort_order), 0) + 1 FROM categories WHERE parent_id = ?1",
                [pid],
                |r| r.get(0),
            )?;
            (parent.kind, next)
        }
        None => {
            let kind = match kind {
                Some(k @ ("income" | "expense")) => k.to_string(),
                _ => {
                    return Err(LedgerError::InvalidInput(
                        "a group needs kind income or expense",
                    ))
                }
            };
            let next: i64 = dbtx.query_row(
                "SELECT COALESCE(MAX(sort_order), 0) + 10 FROM categories WHERE parent_id IS NULL",
                [],
                |r| r.get(0),
            )?;
            (kind, next)
        }
    };
    if name_taken(&dbtx, parent_id, &name, None)? {
        return Err(LedgerError::Conflict(
            "a category with that name already exists",
        ));
    }
    dbtx.execute(
        "INSERT INTO categories (parent_id, name, kind, sort_order, is_system) VALUES (?1, ?2, ?3, ?4, 0)",
        params![parent_id, name, kind, sort_order],
    )?;
    let id = dbtx.last_insert_rowid();
    let created = node(&dbtx, id)?;
    dbtx.commit()?;
    Ok(created)
}

/// Renames, moves (subcategory to another group of the same kind) and recolours.
/// System categories only accept a colour change.
pub fn update_category(
    conn: &Connection,
    id: i64,
    name: &str,
    parent_id: Option<i64>,
    color: Option<&str>,
) -> Result<CategoryNode> {
    check_color(color)?;
    let dbtx = conn.unchecked_transaction()?;
    let cur = load(&dbtx, id)?;
    let new_name = if cur.is_system {
        if name.trim() != cur.name || parent_id != cur.parent_id {
            return Err(LedgerError::InvalidInput(
                "system categories only accept a colour change",
            ));
        }
        cur.name.clone()
    } else {
        let new_name = clean_name(name)?;
        match (cur.parent_id, parent_id) {
            (None, None) => {}
            (None, Some(_)) => {
                return Err(LedgerError::InvalidInput(
                    "a group cannot become a subcategory",
                ))
            }
            (Some(_), None) => {
                return Err(LedgerError::InvalidInput(
                    "a subcategory cannot become a group",
                ))
            }
            (Some(old), Some(new)) if old != new => {
                let target = require_group(&dbtx, new)?;
                if target.kind != cur.kind {
                    return Err(LedgerError::InvalidInput(
                        "a subcategory can only move to a group of the same kind",
                    ));
                }
                let overlap: Option<i64> = dbtx
                    .query_row(
                        "SELECT 1 FROM budgets a JOIN budgets b ON b.category_id = ?2
                         WHERE a.category_id = ?1
                           AND a.start_month <= COALESCE(b.end_month, '9999-99')
                           AND COALESCE(a.end_month, '9999-99') >= b.start_month
                         LIMIT 1",
                        params![id, new],
                        |r| r.get(0),
                    )
                    .optional()?;
                if overlap.is_some() {
                    return Err(LedgerError::Conflict(
                        "the new group has an overlapping budget",
                    ));
                }
            }
            (Some(_), Some(_)) => {}
        }
        if name_taken(&dbtx, parent_id, &new_name, Some(id))? {
            return Err(LedgerError::Conflict(
                "a category with that name already exists",
            ));
        }
        new_name
    };
    dbtx.execute(
        "UPDATE categories SET name = ?1, parent_id = ?2, color = ?3,
                updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE id = ?4",
        params![new_name, parent_id, color, id],
    )?;
    let updated = node(&dbtx, id)?;
    dbtx.commit()?;
    Ok(updated)
}

/// Deletes a category. Its transactions move to `move_to` (any non-transfer category
/// other than itself; `category_source` is kept). Rules are repointed to `move_to` or
/// deleted (always deleted when `move_to` is a system category, which rules cannot target).
/// Its budgets go with it. Refused for system categories and for groups with subcategories.
pub fn delete_category(
    conn: &Connection,
    id: i64,
    move_to: i64,
    delete_rules: bool,
) -> Result<DeleteCategoryResult> {
    let dbtx = conn.unchecked_transaction()?;
    let cat = load(&dbtx, id)?;
    if cat.is_system {
        return Err(LedgerError::InvalidInput(
            "system categories cannot be deleted",
        ));
    }
    let kids: i64 = dbtx.query_row(
        "SELECT count(*) FROM categories WHERE parent_id = ?1",
        [id],
        |r| r.get(0),
    )?;
    if kids > 0 {
        return Err(LedgerError::Conflict("the group still has subcategories"));
    }
    if move_to == id {
        return Err(LedgerError::InvalidInput(
            "the target cannot be the deleted category",
        ));
    }
    let target = load(&dbtx, move_to)?;
    if target.kind == "transfer" {
        return Err(LedgerError::InvalidInput(
            "transactions cannot move to a transfer category",
        ));
    }
    let moved = dbtx.execute(
        "UPDATE transactions SET category_id = ?1 WHERE category_id = ?2",
        params![move_to, id],
    )?;
    let (repointed, deleted_rules) = if delete_rules || target.is_system {
        (
            0,
            dbtx.execute("DELETE FROM rules WHERE category_id = ?1", [id])?,
        )
    } else {
        (
            dbtx.execute(
                "UPDATE rules SET category_id = ?1 WHERE category_id = ?2",
                params![move_to, id],
            )?,
            0,
        )
    };
    let deleted_budgets = dbtx.execute("DELETE FROM budgets WHERE category_id = ?1", [id])?;
    dbtx.execute("DELETE FROM categories WHERE id = ?1", [id])?;
    dbtx.commit()?;
    let n = |v: usize| i64::try_from(v).unwrap_or(i64::MAX);
    Ok(DeleteCategoryResult {
        moved_transactions: n(moved),
        repointed_rules: n(repointed),
        deleted_rules: n(deleted_rules),
        deleted_budgets: n(deleted_budgets),
    })
}
