//! Categorization rules and learning from manual corrections.
//!
//! A rule matches the normalized counterparty and/or description of a
//! transaction. Manual rules beat learned ones; among equals the higher
//! priority, the longer pattern and the newer rule win. When the user corrects
//! a category, `learn_from_correction` stores (or repoints) a learned rule so
//! similar transactions get the same category. Manual and transfer categories
//! are never overwritten by rules.

use rusqlite::{params, Connection, OptionalExtension, Result};

use crate::normalize::{merchant_key, normalize_text};

/// Which text of the transaction a rule looks at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuleField {
    Counterparty,
    Description,
    Any,
}

impl RuleField {
    fn as_str(self) -> &'static str {
        match self {
            RuleField::Counterparty => "counterparty",
            RuleField::Description => "description",
            RuleField::Any => "any",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchType {
    Contains,
    Equals,
}

impl MatchType {
    fn as_str(self) -> &'static str {
        match self {
            MatchType::Contains => "contains",
            MatchType::Equals => "equals",
        }
    }
}

/// `In` = amount > 0, `Out` = amount < 0.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuleDirection {
    Any,
    In,
    Out,
}

impl RuleDirection {
    fn as_str(self) -> &'static str {
        match self {
            RuleDirection::Any => "any",
            RuleDirection::In => "in",
            RuleDirection::Out => "out",
        }
    }

    fn of_amount(amount_cents: i64) -> Self {
        match amount_cents.signum() {
            1 => RuleDirection::In,
            -1 => RuleDirection::Out,
            _ => RuleDirection::Any,
        }
    }
}

/// A rule created by the user.
#[derive(Debug, Clone)]
pub struct NewRule {
    /// Free text; it is normalized before it is stored.
    pub pattern: String,
    pub field: RuleField,
    pub match_type: MatchType,
    pub account_id: Option<i64>,
    pub direction: RuleDirection,
    pub category_id: i64,
    pub priority: i64,
}

impl NewRule {
    /// "Contains" rule on any text, any account, any direction, priority 0.
    pub fn contains(pattern: &str, category_id: i64) -> Self {
        NewRule {
            pattern: pattern.to_string(),
            field: RuleField::Any,
            match_type: MatchType::Contains,
            account_id: None,
            direction: RuleDirection::Any,
            category_id,
            priority: 0,
        }
    }
}

/// Inserts a user rule (origin 'manual') and returns its id. An empty pattern
/// (after normalizing) would match everything, so it is refused with `InvalidQuery`.
pub fn create_manual_rule(conn: &Connection, rule: &NewRule) -> Result<i64> {
    let pattern = normalize_text(&rule.pattern);
    if pattern.is_empty() {
        return Err(rusqlite::Error::InvalidQuery);
    }
    conn.execute(
        "INSERT INTO rules (pattern, field, match_type, account_id, direction, category_id, origin, priority)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'manual', ?7)",
        params![
            pattern,
            rule.field.as_str(),
            rule.match_type.as_str(),
            rule.account_id,
            rule.direction.as_str(),
            rule.category_id,
            rule.priority
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

/// A stored rule, as needed for matching.
struct StoredRule {
    id: i64,
    pattern: String,
    field: String,
    match_type: String,
    account_id: Option<i64>,
    direction: String,
    category_id: i64,
    origin: String,
}

/// A transaction with its texts already normalized.
struct TxView {
    account_id: i64,
    amount_cents: i64,
    category_source: String,
    counterparty: String,
    description: String,
}

/// All rules, best first: manual before learned, then priority, then longer
/// pattern, then newest.
fn load_rules(conn: &Connection) -> Result<Vec<StoredRule>> {
    let mut stmt = conn.prepare(
        "SELECT id, pattern, field, match_type, account_id, direction, category_id, origin
         FROM rules
         ORDER BY (origin = 'manual') DESC, priority DESC, length(pattern) DESC, id DESC",
    )?;
    let rules = stmt
        .query_map([], |r| {
            Ok(StoredRule {
                id: r.get(0)?,
                pattern: r.get(1)?,
                field: r.get(2)?,
                match_type: r.get(3)?,
                account_id: r.get(4)?,
                direction: r.get(5)?,
                category_id: r.get(6)?,
                origin: r.get(7)?,
            })
        })?
        .collect::<Result<Vec<_>>>()?;
    Ok(rules)
}

fn load_view(conn: &Connection, id: i64) -> Result<Option<TxView>> {
    conn.query_row(
        "SELECT account_id, amount_cents, category_source, counterparty, description
         FROM transactions WHERE id = ?1",
        [id],
        |r| {
            Ok(TxView {
                account_id: r.get(0)?,
                amount_cents: r.get(1)?,
                category_source: r.get(2)?,
                counterparty: normalize_text(&r.get::<_, Option<String>>(3)?.unwrap_or_default()),
                description: normalize_text(&r.get::<_, String>(4)?),
            })
        },
    )
    .optional()
}

fn text_matches(rule: &StoredRule, text: &str) -> bool {
    match rule.match_type.as_str() {
        "equals" => text == rule.pattern,
        // Whole-word match on normalized text (single spaces, no punctuation), so
        // "LUNA" matches "CAFETERIA LUNA CENTRO" but not "LUNARES".
        _ => format!(" {text} ").contains(&format!(" {} ", rule.pattern)),
    }
}

fn rule_matches(rule: &StoredRule, tx: &TxView) -> bool {
    if rule.pattern.is_empty() {
        return false;
    }
    if matches!(rule.account_id, Some(a) if a != tx.account_id) {
        return false;
    }
    let direction_ok = match rule.direction.as_str() {
        "in" => tx.amount_cents > 0,
        "out" => tx.amount_cents < 0,
        _ => true,
    };
    if !direction_ok {
        return false;
    }
    match rule.field.as_str() {
        "counterparty" => text_matches(rule, &tx.counterparty),
        "description" => text_matches(rule, &tx.description),
        _ => text_matches(rule, &tx.counterparty) || text_matches(rule, &tx.description),
    }
}

/// Applies the best matching rule to each transaction whose category came from
/// nothing, a rule or learning (manual and transfer categories are untouched).
/// A transaction that matches no rule keeps what it had. Returns how many
/// transactions were given a category by a rule.
pub fn categorize(conn: &Connection, tx_ids: &[i64]) -> Result<usize> {
    let rules = load_rules(conn)?;
    if rules.is_empty() {
        return Ok(0);
    }
    let mut categorized = 0;
    for &id in tx_ids {
        let Some(tx) = load_view(conn, id)? else {
            continue;
        };
        if !matches!(tx.category_source.as_str(), "none" | "rule" | "learned") {
            continue;
        }
        let Some(rule) = rules.iter().find(|r| rule_matches(r, &tx)) else {
            continue;
        };
        let source = if rule.origin == "manual" {
            "rule"
        } else {
            "learned"
        };
        conn.execute(
            "UPDATE transactions SET category_id = ?1, rule_id = ?2, category_source = ?3 WHERE id = ?4",
            params![rule.category_id, rule.id, source, id],
        )?;
        conn.execute(
            "UPDATE rules SET hits = hits + 1, last_hit_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ?1",
            [rule.id],
        )?;
        categorized += 1;
    }
    Ok(categorized)
}

/// Result of learning from one manual correction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LearnOutcome {
    /// The learned rule that was created or repointed, if any.
    pub rule_id: Option<i64>,
    /// Other transactions that received a category because of it.
    pub recategorized: usize,
}

/// The user set `category_id` on a transaction: store it as a manual category
/// and learn a rule from its merchant, so similar transactions follow. The
/// latest correction wins; user-made (manual) rules are never modified.
pub fn learn_from_correction(
    conn: &Connection,
    tx_id: i64,
    category_id: i64,
) -> Result<LearnOutcome> {
    let (amount, description, counterparty, peer): (i64, String, Option<String>, Option<i64>) =
        conn.query_row(
            "SELECT amount_cents, description, counterparty, transfer_peer_id
             FROM transactions WHERE id = ?1",
            [tx_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )?;
    let transfer_cat: i64 = conn.query_row(
        "SELECT id FROM categories WHERE key = 'transfer.internal'",
        [],
        |r| r.get(0),
    )?;
    // Moving one side of a linked transfer to a normal category means it was not
    // an internal transfer: unlink both sides and let the rules re-categorize the peer.
    let unlinked_peer = match peer {
        Some(peer_id) if category_id != transfer_cat => {
            conn.execute(
                "UPDATE transactions
                 SET transfer_peer_id = NULL, category_id = NULL, category_source = 'none', rule_id = NULL
                 WHERE id = ?1 AND category_source = 'transfer'",
                [peer_id],
            )?;
            conn.execute(
                "UPDATE transactions SET transfer_peer_id = NULL WHERE id IN (?1, ?2)",
                params![tx_id, peer_id],
            )?;
            Some(peer_id)
        }
        _ => None,
    };
    conn.execute(
        "UPDATE transactions SET category_id = ?1, category_source = 'manual', rule_id = NULL WHERE id = ?2",
        params![category_id, tx_id],
    )?;
    if let Some(peer_id) = unlinked_peer {
        categorize(conn, &[peer_id])?;
    }

    let Some(key) = merchant_key(&description, counterparty.as_deref()) else {
        return Ok(LearnOutcome {
            rule_id: None,
            recategorized: 0,
        });
    };
    // merchant_key uses the counterparty whenever it has any text.
    let has_counterparty = !normalize_text(counterparty.as_deref().unwrap_or("")).is_empty();
    let field = if has_counterparty {
        RuleField::Counterparty
    } else {
        RuleField::Description
    };
    let direction = RuleDirection::of_amount(amount);

    let existing: Option<(i64, String, i64)> = conn
        .query_row(
            "SELECT id, origin, category_id FROM rules
             WHERE pattern = ?1 AND field = ?2 AND match_type = 'contains'
               AND account_id IS NULL AND direction = ?3",
            params![key, field.as_str(), direction.as_str()],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;

    let rule_id = match existing {
        Some((_, origin, _)) if origin == "manual" => {
            // A rule the user wrote by hand is never changed by learning.
            return Ok(LearnOutcome {
                rule_id: None,
                recategorized: 0,
            });
        }
        Some((id, _, current_category)) => {
            if current_category != category_id {
                conn.execute(
                    "UPDATE rules SET category_id = ?1 WHERE id = ?2",
                    params![category_id, id],
                )?;
            }
            id
        }
        None => {
            conn.execute(
                "INSERT INTO rules (pattern, field, match_type, account_id, direction, category_id, origin)
                 VALUES (?1, ?2, 'contains', NULL, ?3, ?4, 'learned')",
                params![key, field.as_str(), direction.as_str(), category_id],
            )?;
            conn.last_insert_rowid()
        }
    };

    // Other transactions with the same merchant key, not manual and not transfers.
    let mut stmt = conn.prepare(
        "SELECT id, description, counterparty FROM transactions
         WHERE id != ?1 AND category_source IN ('none', 'rule', 'learned')",
    )?;
    let same_merchant: Vec<i64> = stmt
        .query_map([tx_id], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<String>>(2)?,
            ))
        })?
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .filter(|(_, d, c)| merchant_key(d, c.as_deref()).as_deref() == Some(key.as_str()))
        .map(|(id, _, _)| id)
        .collect();
    let recategorized = categorize(conn, &same_merchant)?;
    Ok(LearnOutcome {
        rule_id: Some(rule_id),
        recategorized,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::*;

    fn cat_of(f: &Fixture, id: i64) -> Option<i64> {
        int_of(
            &f.conn,
            "SELECT category_id FROM transactions WHERE id = ?1",
            id,
        )
    }

    fn src_of(f: &Fixture, id: i64) -> String {
        text_of(
            &f.conn,
            "SELECT category_source FROM transactions WHERE id = ?1",
            id,
        )
        .unwrap()
    }

    #[test]
    fn manual_beats_learned_then_priority_then_length_then_newest() {
        let f = fixture();
        let groceries = category_id(&f.conn, "food.groceries");
        let cafes = category_id(&f.conn, "food.cafes");
        let gifts = category_id(&f.conn, "shopping.gifts");
        let tx = raw_booked(
            &f.conn,
            f.own_a,
            "2026-03-01",
            -500,
            "COMPRA SUPERMERCADOS NORTE CAFE",
        );

        // Learned rule with huge priority and a longer pattern still loses to a manual rule.
        f.conn
            .execute(
                "INSERT INTO rules (pattern, field, category_id, origin, priority)
                 VALUES ('SUPERMERCADOS NORTE CAFE', 'any', ?1, 'learned', 99)",
                [gifts],
            )
            .unwrap();
        let short = create_manual_rule(&f.conn, &NewRule::contains("norte", groceries)).unwrap();
        assert_eq!(categorize(&f.conn, &[tx]).unwrap(), 1);
        assert_eq!(
            (cat_of(&f, tx), src_of(&f, tx)),
            (Some(groceries), "rule".to_string())
        );
        assert_eq!(
            int_of(
                &f.conn,
                "SELECT rule_id FROM transactions WHERE id = ?1",
                tx
            ),
            Some(short)
        );
        assert_eq!(
            count(
                &f.conn,
                &format!("SELECT hits FROM rules WHERE id = {short}")
            ),
            1
        );

        // Longer manual pattern wins at equal priority.
        create_manual_rule(&f.conn, &NewRule::contains("supermercados norte", cafes)).unwrap();
        categorize(&f.conn, &[tx]).unwrap();
        assert_eq!(cat_of(&f, tx), Some(cafes));

        // Higher priority wins over a longer pattern.
        let mut high = NewRule::contains("cafe", gifts);
        high.priority = 5;
        create_manual_rule(&f.conn, &high).unwrap();
        categorize(&f.conn, &[tx]).unwrap();
        assert_eq!(cat_of(&f, tx), Some(gifts));
    }

    #[test]
    fn same_priority_and_length_newest_rule_wins() {
        let f = fixture();
        let a = category_id(&f.conn, "food.groceries");
        let b = category_id(&f.conn, "food.cafes");
        let tx = raw_booked(&f.conn, f.own_a, "2026-03-01", -500, "ALPHA BETA");
        create_manual_rule(&f.conn, &NewRule::contains("alpha", a)).unwrap();
        let mut newer = NewRule::contains("beta", b);
        newer.field = RuleField::Description; // different identity, same length class
        create_manual_rule(&f.conn, &newer).unwrap();
        categorize(&f.conn, &[tx]).unwrap();
        // "alpha" has 5 letters and "beta" 4, so the longer pattern is checked first.
        assert_eq!(cat_of(&f, tx), Some(a));
        create_manual_rule(
            &f.conn,
            &NewRule {
                field: RuleField::Counterparty,
                ..NewRule::contains("alpha", b)
            },
        )
        .unwrap();
        f.conn
            .execute(
                "UPDATE transactions SET counterparty = 'Alpha SL' WHERE id = ?1",
                [tx],
            )
            .unwrap();
        categorize(&f.conn, &[tx]).unwrap();
        assert_eq!(cat_of(&f, tx), Some(b)); // same length: the newest id wins
    }

    #[test]
    fn scope_direction_field_and_match_type() {
        let f = fixture();
        let groceries = category_id(&f.conn, "food.groceries");
        let salary = category_id(&f.conn, "income.salary");
        let out_a = raw_booked(&f.conn, f.own_a, "2026-03-01", -500, "SUPERMERCADOS NORTE");
        let out_b = raw_booked(&f.conn, f.own_b, "2026-03-01", -500, "SUPERMERCADOS NORTE");
        let inn_a = raw_booked(&f.conn, f.own_a, "2026-03-01", 500, "SUPERMERCADOS NORTE");

        let mut scoped = NewRule::contains("supermercados norte", groceries);
        scoped.account_id = Some(f.own_a);
        scoped.direction = RuleDirection::Out;
        create_manual_rule(&f.conn, &scoped).unwrap();
        assert_eq!(categorize(&f.conn, &[out_a, out_b, inn_a]).unwrap(), 1);
        assert_eq!(cat_of(&f, out_a), Some(groceries));
        assert_eq!(cat_of(&f, out_b), None);
        assert_eq!(cat_of(&f, inn_a), None);

        // "equals" needs the whole normalized field; "contains" does not.
        let mut eq = NewRule::contains("norte", salary);
        eq.match_type = MatchType::Equals;
        eq.direction = RuleDirection::In;
        create_manual_rule(&f.conn, &eq).unwrap();
        assert_eq!(categorize(&f.conn, &[inn_a]).unwrap(), 0);
        f.conn
            .execute(
                "UPDATE transactions SET counterparty = 'Norte' WHERE id = ?1",
                [inn_a],
            )
            .unwrap();
        assert_eq!(categorize(&f.conn, &[inn_a]).unwrap(), 1);
        assert_eq!(cat_of(&f, inn_a), Some(salary));
    }

    #[test]
    fn manual_and_transfer_categories_are_never_overwritten() {
        let f = fixture();
        let groceries = category_id(&f.conn, "food.groceries");
        let transfer = category_id(&f.conn, "transfer.internal");
        let a = raw_booked(&f.conn, f.own_a, "2026-03-01", -500, "SUPERMERCADOS NORTE");
        let b = raw_booked(&f.conn, f.own_a, "2026-03-01", -500, "SUPERMERCADOS NORTE");
        let free = raw_booked(&f.conn, f.own_a, "2026-03-01", -500, "SUPERMERCADOS NORTE");
        f.conn
            .execute("UPDATE transactions SET category_source = 'manual', category_id = ?1 WHERE id = ?2", [transfer, a])
            .unwrap();
        f.conn
            .execute("UPDATE transactions SET category_source = 'transfer', category_id = ?1 WHERE id = ?2", [transfer, b])
            .unwrap();
        create_manual_rule(&f.conn, &NewRule::contains("norte", groceries)).unwrap();
        assert_eq!(categorize(&f.conn, &[a, b, free]).unwrap(), 1);
        assert_eq!(cat_of(&f, a), Some(transfer));
        assert_eq!(cat_of(&f, b), Some(transfer));
        assert_eq!(cat_of(&f, free), Some(groceries));
    }

    #[test]
    fn no_match_leaves_uncategorized() {
        let f = fixture();
        let tx = raw_booked(&f.conn, f.own_a, "2026-03-01", -500, "ALGO RARO");
        create_manual_rule(
            &f.conn,
            &NewRule::contains("norte", category_id(&f.conn, "food.groceries")),
        )
        .unwrap();
        assert_eq!(categorize(&f.conn, &[tx]).unwrap(), 0);
        assert_eq!((cat_of(&f, tx), src_of(&f, tx)), (None, "none".to_string()));
        assert!(create_manual_rule(&f.conn, &NewRule::contains("  !! ", 1)).is_err());
    }

    #[test]
    fn learning_creates_repoints_and_recategorizes_but_spares_manual() {
        let f = fixture();
        let cafes = category_id(&f.conn, "food.cafes");
        let restaurants = category_id(&f.conn, "food.restaurants");
        let first = raw_booked(
            &f.conn,
            f.own_a,
            "2026-03-01",
            -350,
            "COMPRA TARJ 1234 CAFETERIA LUNA",
        );
        let second = raw_booked(
            &f.conn,
            f.own_a,
            "2026-03-02",
            -400,
            "COMPRA CAFETERIA LUNA 12/03",
        );
        let manual = raw_booked(&f.conn, f.own_a, "2026-03-03", -300, "CAFETERIA LUNA");
        let income = raw_booked(&f.conn, f.own_a, "2026-03-04", 300, "CAFETERIA LUNA");
        let other = raw_booked(&f.conn, f.own_a, "2026-03-05", -300, "SUPERMERCADOS NORTE");
        f.conn
            .execute("UPDATE transactions SET category_source = 'manual', category_id = ?1 WHERE id = ?2", [restaurants, manual])
            .unwrap();

        let out = learn_from_correction(&f.conn, first, cafes).unwrap();
        let rule = out.rule_id.expect("learned rule");
        assert_eq!(out.recategorized, 1);
        assert_eq!(
            (cat_of(&f, first), src_of(&f, first)),
            (Some(cafes), "manual".to_string())
        );
        assert_eq!(
            (cat_of(&f, second), src_of(&f, second)),
            (Some(cafes), "learned".to_string())
        );
        assert_eq!(cat_of(&f, manual), Some(restaurants));
        assert_eq!(cat_of(&f, income), None); // other direction
        assert_eq!(cat_of(&f, other), None);
        assert_eq!(
            text_of(&f.conn, "SELECT pattern FROM rules WHERE id = ?1", rule).as_deref(),
            Some("CAFETERIA LUNA")
        );
        assert_eq!(
            text_of(&f.conn, "SELECT direction FROM rules WHERE id = ?1", rule).as_deref(),
            Some("out")
        );

        // A second correction on the same merchant repoints the same rule.
        let again = learn_from_correction(&f.conn, second, restaurants).unwrap();
        assert_eq!(again.rule_id, Some(rule));
        assert_eq!(count(&f.conn, "SELECT count(*) FROM rules"), 1);
        assert_eq!(
            int_of(&f.conn, "SELECT category_id FROM rules WHERE id = ?1", rule),
            Some(restaurants)
        );
        assert_eq!(cat_of(&f, first), Some(cafes)); // manual stays manual
    }

    #[test]
    fn learning_uses_counterparty_and_never_modifies_manual_rules() {
        let f = fixture();
        let cafes = category_id(&f.conn, "food.cafes");
        let gifts = category_id(&f.conn, "shopping.gifts");
        let tx = raw_booked(&f.conn, f.own_a, "2026-03-01", -350, "COMPRA 99");
        f.conn
            .execute(
                "UPDATE transactions SET counterparty = 'Cafetería Luna' WHERE id = ?1",
                [tx],
            )
            .unwrap();
        let out = learn_from_correction(&f.conn, tx, cafes).unwrap();
        let rule = out.rule_id.unwrap();
        assert_eq!(
            text_of(&f.conn, "SELECT field FROM rules WHERE id = ?1", rule).as_deref(),
            Some("counterparty")
        );

        // A manual rule with the same identity is left alone.
        f.conn.execute("DELETE FROM rules", []).unwrap();
        let mut mine = NewRule::contains("cafeteria luna", gifts);
        mine.field = RuleField::Counterparty;
        mine.direction = RuleDirection::Out;
        let mine_id = create_manual_rule(&f.conn, &mine).unwrap();
        let out = learn_from_correction(&f.conn, tx, cafes).unwrap();
        assert_eq!(out.rule_id, None);
        assert_eq!(
            int_of(
                &f.conn,
                "SELECT category_id FROM rules WHERE id = ?1",
                mine_id
            ),
            Some(gifts)
        );
        assert_eq!(cat_of(&f, tx), Some(cafes)); // the correction itself still applies
    }

    #[test]
    fn learning_without_a_meaningful_key_only_records_the_manual_change() {
        let f = fixture();
        let tx = raw_booked(&f.conn, f.own_a, "2026-03-01", -350, "COMPRA 1234");
        let out = learn_from_correction(&f.conn, tx, category_id(&f.conn, "food.cafes")).unwrap();
        assert_eq!(
            out,
            LearnOutcome {
                rule_id: None,
                recategorized: 0
            }
        );
        assert_eq!(src_of(&f, tx), "manual");
        assert_eq!(count(&f.conn, "SELECT count(*) FROM rules"), 0);
    }

    #[test]
    fn contains_matches_whole_words_only() {
        let f = fixture();
        let cafes = category_id(&f.conn, "food.cafes");
        create_manual_rule(&f.conn, &NewRule::contains("luna", cafes)).unwrap();
        let hit = raw_booked(
            &f.conn,
            f.own_a,
            "2026-03-01",
            -350,
            "CAFETERIA LUNA CENTRO",
        );
        let miss = raw_booked(&f.conn, f.own_a, "2026-03-01", -1999, "TIENDA LUNARES");
        categorize(&f.conn, &[hit, miss]).unwrap();
        assert_eq!(cat_of(&f, hit), Some(cafes));
        assert_eq!(cat_of(&f, miss), None);
    }

    #[test]
    fn correcting_one_side_of_a_transfer_unlinks_the_pair() {
        let f = fixture();
        let out = raw_booked(&f.conn, f.own_a, "2026-03-01", -10000, "TRASPASO DEMO");
        let inn = raw_booked(&f.conn, f.own_b, "2026-03-01", 10000, "TRASPASO DEMO");
        let pairs = crate::transfers::detect_internal_transfers(&f.conn, &[out, inn]).unwrap();
        assert_eq!(pairs.len(), 1);

        let gifts = category_id(&f.conn, "shopping.gifts");
        learn_from_correction(&f.conn, out, gifts).unwrap();

        let peer_of = |id| {
            int_of(
                &f.conn,
                "SELECT transfer_peer_id FROM transactions WHERE id = ?1",
                id,
            )
        };
        assert_eq!(peer_of(out), None);
        assert_eq!(peer_of(inn), None);
        assert_eq!(cat_of(&f, out), Some(gifts));
        assert_ne!(src_of(&f, inn), "transfer");
    }
}
