//! Tests of the Categorías / Reglas / Presupuestos layer. Fictitious data only.

use rusqlite::params;

use super::*;
use crate::test_support::*;

const TODAY: &str = "2026-11-16";

fn cid(f: &Fixture, key: &str) -> i64 {
    category_id(&f.conn, key)
}

/// Booked row on `account` with a category set by hand (`source` = `manual`).
fn tx(f: &Fixture, account: i64, date: &str, cents: i64, text: &str, cat: Option<&str>) -> i64 {
    tx_src(f, account, date, cents, text, cat, "manual")
}

fn tx_src(
    f: &Fixture,
    account: i64,
    date: &str,
    cents: i64,
    text: &str,
    cat: Option<&str>,
    source: &str,
) -> i64 {
    let id = raw_booked(&f.conn, account, date, cents, text);
    if let Some(key) = cat {
        f.conn
            .execute(
                "UPDATE transactions SET category_id = ?1, category_source = ?2 WHERE id = ?3",
                params![cid(f, key), source, id],
            )
            .unwrap();
    }
    id
}

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

fn node(tree: &[CategoryNode], id: i64) -> &CategoryNode {
    tree.iter().find(|n| n.id == id).expect("node")
}

fn make_pending(f: &Fixture, id: i64) {
    f.conn
        .execute(
            "UPDATE transactions SET status = 'pending', value_date = booking_date, booking_date = NULL WHERE id = ?1",
            [id],
        )
        .unwrap();
}

// ---- categories ----

#[test]
fn tree_aggregates_groups_transfers_and_budget_flags() {
    let f = fixture();
    tx(
        &f,
        f.own_a,
        "2026-10-03",
        -5000,
        "super",
        Some("food.groceries"),
    );
    tx(&f, f.own_a, "2026-10-04", -1500, "cafe", Some("food.cafes"));
    tx(&f, f.own_a, "2026-10-05", -200, "directo", Some("food"));
    tx(
        &f,
        f.own_a,
        "2026-09-30",
        -999,
        "septiembre",
        Some("food.groceries"),
    );
    tx(
        &f,
        f.external,
        "2026-10-06",
        -777,
        "externa",
        Some("food.groceries"),
    );
    let pending = tx(
        &f,
        f.own_a,
        "2026-10-07",
        -100,
        "pendiente",
        Some("food.cafes"),
    );
    make_pending(&f, pending);
    tx_src(
        &f,
        f.own_a,
        "2026-10-08",
        -30000,
        "traspaso",
        Some("transfer.internal"),
        "transfer",
    );
    tx_src(
        &f,
        f.own_b,
        "2026-10-08",
        30000,
        "traspaso",
        Some("transfer.internal"),
        "transfer",
    );
    set_budget(&f.conn, cid(&f, "food"), "2026-01", 40000).unwrap();
    set_budget(&f.conn, cid(&f, "transport.fuel"), "2026-01", 9000).unwrap();
    let rule = crate::rules::NewRule::contains("cafe", cid(&f, "food.cafes"));
    crate::rules::create_manual_rule(&f.conn, &rule).unwrap();

    let tree = category_tree(&f.conn, "2026-10").unwrap();
    let food = node(&tree, cid(&f, "food"));
    assert_eq!(food.child_count, 3);
    assert_eq!(food.month_tx_count, 4);
    assert_eq!(food.month_amount_cents, -6800);
    assert_eq!(food.total_tx_count, 6); // all history, all accounts
    assert_eq!(food.rule_count, 1);
    assert_eq!(food.budget, Some("own"));
    let groceries = node(&tree, cid(&f, "food.groceries"));
    assert_eq!(
        (groceries.month_tx_count, groceries.month_amount_cents),
        (1, -5000)
    );
    assert_eq!(groceries.budget, Some("group"));
    assert_eq!(node(&tree, cid(&f, "transport")).budget, Some("children"));
    assert_eq!(node(&tree, cid(&f, "utilities")).budget, None);
    let transfer = node(&tree, cid(&f, "transfer.internal"));
    assert_eq!(
        (transfer.month_tx_count, transfer.month_amount_cents),
        (2, 30000)
    );
    assert!(transfer.is_system);
    // Order: each group is followed by its children.
    let pos = |key: &str| tree.iter().position(|n| n.id == cid(&f, key)).unwrap();
    assert!(pos("food") < pos("food.groceries") && pos("food.cafes") < pos("transport"));
    // Another month: no month figures, no budget flag.
    let other = category_tree(&f.conn, "2025-12").unwrap();
    assert_eq!(node(&other, cid(&f, "food")).month_tx_count, 0);
    assert_eq!(node(&other, cid(&f, "food")).budget, None);
    assert!(category_tree(&f.conn, "2026-13").is_err());
}

#[test]
fn create_rename_move_and_colour() {
    let f = fixture();
    let group = create_category(&f.conn, None, "  Mascotas ", Some("expense")).unwrap();
    assert_eq!(
        (group.name.as_str(), group.kind.as_str(), group.parent_id),
        ("Mascotas", "expense", None)
    );
    let sub = create_category(&f.conn, Some(group.id), "Veterinario", Some("income")).unwrap();
    assert_eq!(sub.kind, "expense"); // inherited, argument ignored
                                     // Duplicates: case-insensitive per parent, allowed under another parent.
    assert!(matches!(
        create_category(&f.conn, None, "mascotas", Some("expense")),
        Err(LedgerError::Conflict(_))
    ));
    assert!(matches!(
        create_category(&f.conn, Some(group.id), "VETERINARIO", None),
        Err(LedgerError::Conflict(_))
    ));
    assert!(create_category(&f.conn, Some(cid(&f, "food")), "Veterinario", None).is_ok());
    // Validation.
    assert!(matches!(
        create_category(&f.conn, None, "   ", Some("expense")),
        Err(LedgerError::InvalidInput(_))
    ));
    assert!(matches!(
        create_category(&f.conn, None, &"x".repeat(61), Some("expense")),
        Err(LedgerError::InvalidInput(_))
    ));
    assert!(create_category(&f.conn, None, &"x".repeat(60), Some("expense")).is_ok());
    assert!(matches!(
        create_category(&f.conn, None, "Sin tipo", None),
        Err(LedgerError::InvalidInput(_))
    ));
    assert!(matches!(
        create_category(&f.conn, None, "Mal tipo", Some("transfer")),
        Err(LedgerError::InvalidInput(_))
    ));
    assert!(matches!(
        create_category(&f.conn, Some(sub.id), "Nieto", None),
        Err(LedgerError::InvalidInput(_))
    ));
    assert!(matches!(
        create_category(&f.conn, Some(cid(&f, "uncategorized")), "Hija", None),
        Err(LedgerError::InvalidInput(_))
    ));
    assert!(matches!(
        create_category(&f.conn, Some(999_999), "Huérfana", None),
        Err(LedgerError::NotFound(_))
    ));
    // New groups go after the existing ones.
    let tree = category_tree(&f.conn, "2026-10").unwrap();
    let max_group = tree
        .iter()
        .filter(|n| n.parent_id.is_none())
        .map(|n| n.sort_order)
        .max()
        .unwrap();
    assert_eq!(
        node(
            &tree,
            tree.iter().find(|n| n.name == "x".repeat(60)).unwrap().id
        )
        .sort_order,
        max_group
    );

    // Rename + colour + move to another group of the same kind.
    let health = cid(&f, "health");
    let moved =
        update_category(&f.conn, sub.id, "Veterinaria", Some(health), Some("cat-3")).unwrap();
    assert_eq!(
        (moved.name.as_str(), moved.parent_id, moved.color.as_deref()),
        ("Veterinaria", Some(health), Some("cat-3"))
    );
    let g = update_category(&f.conn, group.id, "Animales", None, Some("other")).unwrap();
    assert_eq!(
        (g.name.as_str(), g.color.as_deref()),
        ("Animales", Some("other"))
    );
    assert_eq!(
        update_category(&f.conn, group.id, "Animales", None, None)
            .unwrap()
            .color,
        None
    );
    assert!(matches!(
        update_category(&f.conn, group.id, "Animales", None, Some("rojo")),
        Err(LedgerError::InvalidInput(_))
    ));
    // Refused moves.
    let income_group = cid(&f, "income");
    assert!(matches!(
        update_category(&f.conn, moved.id, "Veterinaria", Some(income_group), None),
        Err(LedgerError::InvalidInput(_))
    ));
    assert!(matches!(
        update_category(&f.conn, moved.id, "Veterinaria", None, None),
        Err(LedgerError::InvalidInput(_))
    ));
    assert!(matches!(
        update_category(&f.conn, group.id, "Animales", Some(cid(&f, "food")), None),
        Err(LedgerError::InvalidInput(_))
    ));
    assert!(matches!(
        update_category(
            &f.conn,
            moved.id,
            "Veterinaria",
            Some(cid(&f, "food.cafes")),
            None
        ),
        Err(LedgerError::InvalidInput(_))
    ));
    assert!(matches!(
        update_category(
            &f.conn,
            moved.id,
            "Veterinaria",
            Some(cid(&f, "transfer.internal")),
            None
        ),
        Err(LedgerError::InvalidInput(_))
    ));
    // Rename onto a sibling; own name in another case is fine.
    assert!(matches!(
        update_category(&f.conn, moved.id, "farmacia", Some(health), None),
        Err(LedgerError::Conflict(_))
    ));
    assert!(update_category(&f.conn, moved.id, "VETERINARIA", Some(health), None).is_ok());
    assert!(matches!(
        update_category(&f.conn, 999_999, "x", None, None),
        Err(LedgerError::NotFound(_))
    ));
}

#[test]
fn moving_a_subcategory_under_a_group_with_an_overlapping_budget_conflicts() {
    let f = fixture();
    let sub = create_category(&f.conn, Some(cid(&f, "leisure")), "Juegos", None).unwrap();
    set_budget(&f.conn, sub.id, "2026-03", 1000).unwrap();
    set_budget(&f.conn, cid(&f, "shopping"), "2026-05", 5000).unwrap();
    assert!(matches!(
        update_category(&f.conn, sub.id, "Juegos", Some(cid(&f, "shopping")), None),
        Err(LedgerError::Conflict(_))
    ));
    // No overlap once the subcategory budget ended before the group one starts.
    remove_budget(&f.conn, sub.id, "2026-04").unwrap();
    assert!(update_category(&f.conn, sub.id, "Juegos", Some(cid(&f, "shopping")), None).is_ok());
}

#[test]
fn system_categories_only_change_colour() {
    let f = fixture();
    for key in ["transfer.internal", "uncategorized"] {
        let id = cid(&f, key);
        let name: String = f
            .conn
            .query_row("SELECT name FROM categories WHERE id = ?1", [id], |r| {
                r.get(0)
            })
            .unwrap();
        let coloured = update_category(&f.conn, id, &name, None, Some("cat-5")).unwrap();
        assert_eq!(coloured.color.as_deref(), Some("cat-5"));
        assert!(matches!(
            update_category(&f.conn, id, "Otro nombre", None, None),
            Err(LedgerError::InvalidInput(_))
        ));
        assert!(matches!(
            update_category(&f.conn, id, &name, Some(cid(&f, "food")), None),
            Err(LedgerError::InvalidInput(_))
        ));
        assert!(matches!(
            create_category(&f.conn, Some(id), "Hija", None),
            Err(LedgerError::InvalidInput(_))
        ));
        assert!(matches!(
            delete_category(&f.conn, id, cid(&f, "food"), false),
            Err(LedgerError::InvalidInput(_))
        ));
    }
}

#[test]
fn delete_repoints_rules_and_keeps_category_source() {
    let f = fixture();
    let cafes = cid(&f, "food.cafes");
    let food = cid(&f, "food");
    let manual = tx(
        &f,
        f.own_a,
        "2026-10-01",
        -300,
        "cafe uno",
        Some("food.cafes"),
    );
    let by_rule = tx_src(
        &f,
        f.own_a,
        "2026-10-02",
        -300,
        "cafe dos",
        Some("food.cafes"),
        "rule",
    );
    crate::rules::create_manual_rule(&f.conn, &crate::rules::NewRule::contains("cafe", cafes))
        .unwrap();
    set_budget(&f.conn, cafes, "2026-01", 1000).unwrap();
    let tree = category_tree(&f.conn, "2026-10").unwrap();
    assert_eq!(node(&tree, cafes).total_tx_count, 2);

    let res = delete_category(&f.conn, cafes, food, false).unwrap();
    assert_eq!(
        res,
        DeleteCategoryResult {
            moved_transactions: 2,
            repointed_rules: 1,
            deleted_rules: 0,
            deleted_budgets: 1
        }
    );
    assert_eq!(
        (cat_of(&f, manual), cat_of(&f, by_rule)),
        (Some(food), Some(food))
    );
    assert_eq!(
        (src_of(&f, manual).as_str(), src_of(&f, by_rule).as_str()),
        ("manual", "rule")
    );
    assert_eq!(list_rules(&f.conn).unwrap()[0].category_id, food);
    assert_eq!(count(&f.conn, "SELECT count(*) FROM budgets"), 0);
    assert_eq!(
        count(
            &f.conn,
            &format!("SELECT count(*) FROM categories WHERE id = {cafes}")
        ),
        0
    );
}

#[test]
fn delete_can_drop_the_rules_and_system_targets_force_it() {
    let f = fixture();
    let fuel = cid(&f, "transport.fuel");
    let taxi = cid(&f, "transport.taxi");
    let t = tx_src(
        &f,
        f.own_a,
        "2026-10-01",
        -4000,
        "gasolinera",
        Some("transport.fuel"),
        "rule",
    );
    let rule = crate::rules::create_manual_rule(
        &f.conn,
        &crate::rules::NewRule::contains("gasolinera", fuel),
    )
    .unwrap();
    f.conn
        .execute(
            "UPDATE transactions SET rule_id = ?1 WHERE id = ?2",
            params![rule, t],
        )
        .unwrap();
    let res = delete_category(&f.conn, fuel, taxi, true).unwrap();
    assert_eq!(
        (
            res.moved_transactions,
            res.repointed_rules,
            res.deleted_rules
        ),
        (1, 0, 1)
    );
    assert_eq!(cat_of(&f, t), Some(taxi));
    assert_eq!(src_of(&f, t), "rule");
    assert_eq!(count(&f.conn, "SELECT count(*) FROM rules"), 0);
    assert_eq!(
        int_of(&f.conn, "SELECT rule_id FROM transactions WHERE id = ?1", t),
        None
    );

    // Moving to "Sin categoría": rules cannot follow, so they are deleted either way.
    let uncategorized = cid(&f, "uncategorized");
    crate::rules::create_manual_rule(&f.conn, &crate::rules::NewRule::contains("taxi", taxi))
        .unwrap();
    let res = delete_category(&f.conn, taxi, uncategorized, false).unwrap();
    assert_eq!(
        (
            res.moved_transactions,
            res.repointed_rules,
            res.deleted_rules
        ),
        (1, 0, 1)
    );
    assert_eq!(cat_of(&f, t), Some(uncategorized));
}

#[test]
fn delete_refusals() {
    let f = fixture();
    let food = cid(&f, "food");
    let health = cid(&f, "health");
    assert!(matches!(
        delete_category(&f.conn, food, health, false),
        Err(LedgerError::Conflict(_))
    ));
    let cafes = cid(&f, "food.cafes");
    assert!(matches!(
        delete_category(&f.conn, cafes, cafes, false),
        Err(LedgerError::InvalidInput(_))
    ));
    assert!(matches!(
        delete_category(&f.conn, cafes, cid(&f, "transfer.internal"), false),
        Err(LedgerError::InvalidInput(_))
    ));
    assert!(matches!(
        delete_category(&f.conn, cafes, 999_999, false),
        Err(LedgerError::NotFound(_))
    ));
    assert!(matches!(
        delete_category(&f.conn, 999_999, health, false),
        Err(LedgerError::NotFound(_))
    ));
    let empty = create_category(&f.conn, None, "Vacía", Some("expense")).unwrap();
    assert!(delete_category(&f.conn, empty.id, health, false).is_ok());
}

// ---- rules ----

fn input(pattern: &str, category: i64) -> RuleInput {
    RuleInput {
        pattern: pattern.to_string(),
        field: "any".into(),
        match_type: "contains".into(),
        account_id: None,
        direction: "any".into(),
        category_id: category,
        priority: 0,
    }
}

#[test]
fn rules_list_in_evaluation_order_and_editing_a_learned_rule_makes_it_manual() {
    let f = fixture();
    let cafes = cid(&f, "food.cafes");
    let groceries = cid(&f, "food.groceries");
    let learned_tx = tx(&f, f.own_a, "2026-10-01", -450, "Panaderia Sol", None);
    crate::rules::learn_from_correction(&f.conn, learned_tx, cafes).unwrap();
    let low = save_rule(
        &f.conn,
        None,
        &RuleInput {
            priority: -10,
            ..input("super", groceries)
        },
        false,
        TODAY,
    )
    .unwrap();
    let high = save_rule(
        &f.conn,
        None,
        &RuleInput {
            priority: 10,
            ..input("a", cafes)
        },
        false,
        TODAY,
    )
    .unwrap();
    let normal = save_rule(&f.conn, None, &input("mercado", groceries), false, TODAY).unwrap();
    assert_eq!(low.rule.origin, "manual");

    let rules = list_rules(&f.conn).unwrap();
    let ids: Vec<i64> = rules.iter().map(|r| r.id).collect();
    assert_eq!(ids[..3], [high.rule.id, normal.rule.id, low.rule.id]);
    assert_eq!(
        rules.iter().map(|r| r.order).collect::<Vec<_>>(),
        vec![1, 2, 3, 4]
    );
    let learned = &rules[3];
    assert_eq!(learned.origin, "learned");
    assert_eq!(
        (
            learned.category_name.as_str(),
            learned.parent_category_name.as_deref()
        ),
        ("Cafeterías", Some("Alimentación"))
    );

    let edited = save_rule(
        &f.conn,
        Some(learned.id),
        &input(&learned.pattern, groceries),
        false,
        TODAY,
    )
    .unwrap();
    assert_eq!(edited.rule.origin, "manual");
    assert_eq!(edited.rule.category_id, groceries);
    assert!(edited.rule.order <= 3);
    let n = save_rule(
        &f.conn,
        None,
        &input("  CAFÉ   Ñandú!! ", cafes),
        false,
        TODAY,
    )
    .unwrap();
    assert_eq!(n.rule.pattern, "CAFE NANDU");
}

#[test]
fn rule_duplicates_and_validation() {
    let f = fixture();
    let cafes = cid(&f, "food.cafes");
    let first = save_rule(&f.conn, None, &input("luna", cafes), false, TODAY).unwrap();
    assert!(matches!(
        save_rule(
            &f.conn,
            None,
            &input("LUNA", cid(&f, "food.groceries")),
            false,
            TODAY
        ),
        Err(LedgerError::Conflict(_))
    ));
    let out = RuleInput {
        direction: "out".into(),
        ..input("luna", cafes)
    };
    let second = save_rule(&f.conn, None, &out, false, TODAY).unwrap();
    let acct = RuleInput {
        account_id: Some(f.own_a),
        ..input("luna", cafes)
    };
    assert!(save_rule(&f.conn, None, &acct, false, TODAY).is_ok());
    assert!(matches!(
        save_rule(
            &f.conn,
            Some(second.rule.id),
            &input("luna", cafes),
            false,
            TODAY
        ),
        Err(LedgerError::Conflict(_))
    ));
    assert!(save_rule(
        &f.conn,
        Some(first.rule.id),
        &input("luna", cafes),
        false,
        TODAY
    )
    .is_ok());

    let bad = |r: RuleInput| save_rule(&f.conn, None, &r, false, TODAY);
    assert!(matches!(
        bad(input("   !! ", cafes)),
        Err(LedgerError::InvalidInput(_))
    ));
    assert!(matches!(
        bad(input(&"a".repeat(101), cafes)),
        Err(LedgerError::InvalidInput(_))
    ));
    assert!(bad(input(&"b".repeat(100), cafes)).is_ok());
    assert!(matches!(
        bad(RuleInput {
            field: "x".into(),
            ..input("z1", cafes)
        }),
        Err(LedgerError::InvalidInput(_))
    ));
    assert!(matches!(
        bad(RuleInput {
            match_type: "regex".into(),
            ..input("z2", cafes)
        }),
        Err(LedgerError::InvalidInput(_))
    ));
    assert!(matches!(
        bad(RuleInput {
            direction: "up".into(),
            ..input("z3", cafes)
        }),
        Err(LedgerError::InvalidInput(_))
    ));
    assert!(matches!(
        bad(RuleInput {
            account_id: Some(999_999),
            ..input("z4", cafes)
        }),
        Err(LedgerError::InvalidInput(_))
    ));
    assert!(matches!(
        bad(input("z5", cid(&f, "transfer.internal"))),
        Err(LedgerError::InvalidInput(_))
    ));
    assert!(matches!(
        bad(input("z6", cid(&f, "uncategorized"))),
        Err(LedgerError::InvalidInput(_))
    ));
    assert!(matches!(
        bad(input("z7", 999_999)),
        Err(LedgerError::NotFound(_))
    ));
    assert!(matches!(
        save_rule(&f.conn, Some(999_999), &input("z8", cafes), false, TODAY),
        Err(LedgerError::NotFound(_))
    ));
    assert!(matches!(
        save_rule(&f.conn, None, &input("z9", cafes), false, "16/11/2026"),
        Err(LedgerError::InvalidInput(_))
    ));
    assert!(bad(input("nomina", cid(&f, "income.salary"))).is_ok());
}

#[test]
fn deleting_a_rule_keeps_the_categories_it_set() {
    let f = fixture();
    let cafes = cid(&f, "food.cafes");
    let t = tx_src(
        &f,
        f.own_a,
        "2026-11-01",
        -300,
        "cafeteria luna",
        None,
        "none",
    );
    let saved = save_rule(&f.conn, None, &input("luna", cafes), true, TODAY).unwrap();
    assert_eq!(saved.recategorized, 1);
    assert_eq!(
        int_of(&f.conn, "SELECT rule_id FROM transactions WHERE id = ?1", t),
        Some(saved.rule.id)
    );
    delete_rule(&f.conn, saved.rule.id).unwrap();
    assert_eq!(cat_of(&f, t), Some(cafes));
    assert_eq!(
        int_of(&f.conn, "SELECT rule_id FROM transactions WHERE id = ?1", t),
        None
    );
    assert!(matches!(
        delete_rule(&f.conn, saved.rule.id),
        Err(LedgerError::NotFound(_))
    ));
}

#[test]
fn test_rule_reports_each_outcome() {
    let f = fixture();
    let cafes = cid(&f, "food.cafes");
    let groceries = cid(&f, "food.groceries");
    let earlier = save_rule(
        &f.conn,
        None,
        &RuleInput {
            priority: 5,
            ..input("luna centro", groceries)
        },
        false,
        TODAY,
    )
    .unwrap()
    .rule;
    let change = tx_src(
        &f,
        f.own_a,
        "2026-11-10",
        -300,
        "Cafeteria Luna",
        None,
        "none",
    );
    let unchanged = tx_src(
        &f,
        f.own_a,
        "2026-11-09",
        -310,
        "Cafeteria Luna 2",
        Some("food.cafes"),
        "rule",
    );
    let manual = tx(
        &f,
        f.own_a,
        "2026-11-08",
        -320,
        "Cafeteria Luna 3",
        Some("food.groceries"),
    );
    let transfer = tx_src(
        &f,
        f.own_b,
        "2026-11-07",
        -330,
        "Luna traspaso",
        Some("transfer.internal"),
        "transfer",
    );
    let other = tx_src(
        &f,
        f.own_a,
        "2026-11-06",
        -340,
        "Cafeteria Luna Centro",
        None,
        "none",
    );
    tx_src(
        &f,
        f.own_a,
        "2026-08-18",
        -350,
        "Luna antigua",
        None,
        "none",
    ); // day 91: outside
    let edge = tx_src(&f, f.own_a, "2026-08-19", -360, "Luna limite", None, "none"); // day 90: inside
    tx_src(
        &f,
        f.external,
        "2026-11-05",
        -370,
        "Luna externa",
        None,
        "none",
    );
    tx_src(&f, f.own_a, "2026-11-04", -380, "Lunares", None, "none");

    let t = test_rule(&f.conn, None, &input("luna", cafes), TODAY).unwrap();
    assert_eq!((t.from.as_str(), t.to.as_str()), ("2026-08-19", TODAY));
    assert_eq!(
        (t.matched, t.change, t.unchanged, t.manual, t.other_rule),
        (6, 2, 1, 2, 1)
    );
    let outcomes: Vec<(i64, &str)> = t.rows.iter().map(|r| (r.tx_id, r.outcome)).collect();
    assert_eq!(outcomes[..2], [(change, "change"), (edge, "change")]);
    assert_eq!(outcomes[2], (other, "other_rule"));
    assert_eq!(outcomes[3..5], [(manual, "manual"), (transfer, "manual")]);
    assert_eq!(outcomes[5], (unchanged, "unchanged"));
    let o = &t.rows[2];
    assert_eq!(o.winning_rule_id, Some(earlier.id));
    assert_eq!(o.winning_rule_pattern.as_deref(), Some("LUNA CENTRO"));
    assert_eq!(o.winning_rule_order, Some(1));
    assert_eq!(t.rows[0].winning_rule_id, None);
    assert_eq!(t.rows[0].category_id, None);
    assert_eq!(t.rows[5].category_name.as_deref(), Some("Cafeterías"));
    // Nothing was written.
    assert_eq!(cat_of(&f, change), None);
    assert_eq!(count(&f.conn, "SELECT count(*) FROM rules"), 1);

    // Testing an existing rule replaces it: a higher priority wins over the earlier rule.
    let luna = save_rule(&f.conn, None, &input("luna", cafes), false, TODAY)
        .unwrap()
        .rule;
    assert_eq!(luna.order, 2);
    let t2 = test_rule(
        &f.conn,
        Some(luna.id),
        &RuleInput {
            priority: 10,
            ..input("luna", cafes)
        },
        TODAY,
    )
    .unwrap();
    assert_eq!((t2.other_rule, t2.change), (0, 3));
    let incoming = RuleInput {
        direction: "in".into(),
        ..input("luna", cafes)
    };
    assert_eq!(
        test_rule(&f.conn, None, &incoming, TODAY).unwrap().matched,
        0
    );
    let on_b = RuleInput {
        account_id: Some(f.own_b),
        ..input("luna", cafes)
    };
    assert_eq!(test_rule(&f.conn, None, &on_b, TODAY).unwrap().matched, 1);
    assert!(matches!(
        test_rule(&f.conn, Some(999_999), &input("luna", cafes), TODAY),
        Err(LedgerError::NotFound(_))
    ));
}

#[test]
fn test_rule_caps_the_rows_at_fifty() {
    let f = fixture();
    for i in 0..60 {
        tx_src(&f, f.own_a, "2026-11-01", -100 - i, "Luna", None, "none");
    }
    let t = test_rule(&f.conn, None, &input("luna", cid(&f, "food.cafes")), TODAY).unwrap();
    assert_eq!((t.matched, t.change, t.rows.len()), (60, 60, 50));
}

#[test]
fn apply_on_save_only_touches_free_rows_inside_the_window() {
    let f = fixture();
    let cafes = cid(&f, "food.cafes");
    let none = tx_src(
        &f,
        f.own_a,
        "2026-11-10",
        -300,
        "Cafeteria Luna",
        None,
        "none",
    );
    let learned = tx_src(
        &f,
        f.own_a,
        "2026-10-10",
        -300,
        "Cafeteria Luna",
        Some("food.groceries"),
        "learned",
    );
    let by_rule = tx_src(
        &f,
        f.own_a,
        "2026-10-11",
        -300,
        "Cafeteria Luna",
        Some("food.cafes"),
        "rule",
    );
    let manual = tx(
        &f,
        f.own_a,
        "2026-11-09",
        -300,
        "Cafeteria Luna",
        Some("food.groceries"),
    );
    let transfer = tx_src(
        &f,
        f.own_a,
        "2026-11-08",
        -300,
        "Luna",
        Some("transfer.internal"),
        "transfer",
    );
    let old = tx_src(
        &f,
        f.own_a,
        "2026-05-01",
        -300,
        "Cafeteria Luna",
        None,
        "none",
    );
    let unrelated = tx_src(&f, f.own_a, "2026-11-07", -300, "Gasolinera", None, "none");

    let saved = save_rule(&f.conn, None, &input("luna", cafes), true, TODAY).unwrap();
    assert_eq!(saved.recategorized, 2); // `none` and `learned`; `by_rule` was already there
    assert_eq!(
        (cat_of(&f, none), src_of(&f, none).as_str()),
        (Some(cafes), "rule")
    );
    assert_eq!(
        (cat_of(&f, learned), src_of(&f, learned).as_str()),
        (Some(cafes), "rule")
    );
    assert_eq!(cat_of(&f, by_rule), Some(cafes));
    assert_eq!(cat_of(&f, manual), Some(cid(&f, "food.groceries")));
    assert_eq!(src_of(&f, manual), "manual");
    assert_eq!(cat_of(&f, transfer), Some(cid(&f, "transfer.internal")));
    assert_eq!(cat_of(&f, old), None);
    assert_eq!(cat_of(&f, unrelated), None);

    let t2 = tx_src(
        &f,
        f.own_a,
        "2026-11-12",
        -300,
        "Panaderia Sol",
        None,
        "none",
    );
    let saved = save_rule(&f.conn, None, &input("sol", cafes), false, TODAY).unwrap();
    assert_eq!(saved.recategorized, 0);
    assert_eq!(cat_of(&f, t2), None);
}

#[test]
fn apply_rules_to_uncategorized_covers_all_history_and_skips_manual_rows() {
    let f = fixture();
    let cafes = cid(&f, "food.cafes");
    save_rule(&f.conn, None, &input("luna", cafes), false, TODAY).unwrap();
    let none_old = tx_src(
        &f,
        f.own_a,
        "2020-01-15",
        -300,
        "Cafeteria Luna",
        None,
        "none",
    );
    let in_uncat = tx_src(
        &f,
        f.own_a,
        "2026-11-01",
        -300,
        "Cafeteria Luna",
        Some("uncategorized"),
        "rule",
    );
    let manual_uncat = tx(
        &f,
        f.own_a,
        "2026-11-02",
        -300,
        "Cafeteria Luna",
        Some("uncategorized"),
    );
    let categorized = tx_src(
        &f,
        f.own_a,
        "2026-11-03",
        -300,
        "Cafeteria Luna",
        Some("food.groceries"),
        "rule",
    );
    let external = tx_src(
        &f,
        f.external,
        "2026-11-04",
        -300,
        "Cafeteria Luna",
        None,
        "none",
    );
    let no_match = tx_src(&f, f.own_a, "2026-11-05", -300, "Otra cosa", None, "none");

    assert_eq!(
        apply_rules_uncategorized(&f.conn).unwrap(),
        ApplyResult { recategorized: 2 }
    );
    assert_eq!(cat_of(&f, none_old), Some(cafes));
    assert_eq!(cat_of(&f, in_uncat), Some(cafes));
    assert_eq!(cat_of(&f, manual_uncat), Some(cid(&f, "uncategorized")));
    assert_eq!(cat_of(&f, categorized), Some(cid(&f, "food.groceries")));
    assert_eq!(cat_of(&f, external), None);
    assert_eq!(cat_of(&f, no_match), None);
    assert_eq!(apply_rules_uncategorized(&f.conn).unwrap().recategorized, 0);
}

// ---- budgets ----

type BudgetRowDb = (String, Option<String>, i64);

fn rows_of(f: &Fixture, key: &str) -> Vec<BudgetRowDb> {
    let mut stmt = f
        .conn
        .prepare("SELECT start_month, end_month, amount_cents FROM budgets WHERE category_id = ?1 ORDER BY start_month")
        .unwrap();
    let rows = stmt
        .query_map([cid(f, key)], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    rows
}

fn month_budgets(f: &Fixture, month: &str) -> BudgetMonth {
    budgets(&f.conn, month, "2026-10-20").unwrap()
}

fn s(v: &str) -> String {
    v.to_string()
}

#[test]
fn set_budget_updates_a_row_that_starts_then_and_splits_an_older_one() {
    let f = fixture();
    let g = cid(&f, "food.groceries");
    set_budget(&f.conn, g, "2026-03", 30000).unwrap();
    set_budget(&f.conn, g, "2026-03", 35000).unwrap();
    assert_eq!(
        rows_of(&f, "food.groceries"),
        vec![(s("2026-03"), None, 35000)]
    );
    set_budget(&f.conn, g, "2026-06", 40000).unwrap();
    assert_eq!(
        rows_of(&f, "food.groceries"),
        vec![
            (s("2026-03"), Some(s("2026-05")), 35000),
            (s("2026-06"), None, 40000)
        ]
    );
    let april = month_budgets(&f, "2026-04");
    let r = &april.budgets[0];
    assert_eq!(
        (
            r.amount_cents,
            r.start_month.as_str(),
            r.previous_amount_cents
        ),
        (35000, "2026-03", None)
    );
    let july = month_budgets(&f, "2026-07");
    let r = &july.budgets[0];
    assert_eq!(
        (
            r.amount_cents,
            r.start_month.as_str(),
            r.previous_amount_cents
        ),
        (40000, "2026-06", Some(35000))
    );
    assert!(month_budgets(&f, "2026-02").budgets.is_empty());
    // Setting inside a closed span keeps the chain consistent.
    set_budget(&f.conn, g, "2026-04", 36000).unwrap();
    assert_eq!(
        rows_of(&f, "food.groceries"),
        vec![
            (s("2026-03"), Some(s("2026-03")), 35000),
            (s("2026-04"), Some(s("2026-05")), 36000),
            (s("2026-06"), None, 40000)
        ]
    );
    // Across the year boundary.
    set_budget(&f.conn, cid(&f, "transport.fuel"), "2026-12", 5000).unwrap();
    set_budget(&f.conn, cid(&f, "transport.fuel"), "2027-01", 6000).unwrap();
    assert_eq!(rows_of(&f, "transport.fuel")[0].1, Some(s("2026-12")));
}

#[test]
fn set_budget_validation() {
    let f = fixture();
    let g = cid(&f, "food.groceries");
    assert!(matches!(
        set_budget(&f.conn, g, "2026-03", 0),
        Err(LedgerError::InvalidInput(_))
    ));
    assert!(matches!(
        set_budget(&f.conn, g, "2026-03", -5),
        Err(LedgerError::InvalidInput(_))
    ));
    assert!(matches!(
        set_budget(&f.conn, g, "2026-03", 100_000_001),
        Err(LedgerError::InvalidInput(_))
    ));
    assert!(set_budget(&f.conn, g, "2026-03", 100_000_000).is_ok());
    assert!(set_budget(&f.conn, g, "2026-03", 1).is_ok());
    assert!(matches!(
        set_budget(&f.conn, g, "2026-3", 100),
        Err(LedgerError::InvalidInput(_))
    ));
    assert!(matches!(
        set_budget(&f.conn, g, "2026-13", 100),
        Err(LedgerError::InvalidInput(_))
    ));
    for key in [
        "income",
        "income.salary",
        "transfer.internal",
        "uncategorized",
    ] {
        assert!(
            matches!(
                set_budget(&f.conn, cid(&f, key), "2026-03", 100),
                Err(LedgerError::InvalidInput(_))
            ),
            "{key}"
        );
    }
    assert!(matches!(
        set_budget(&f.conn, 999_999, "2026-03", 100),
        Err(LedgerError::NotFound(_))
    ));
}

#[test]
fn remove_budget_deletes_a_row_that_starts_then_and_closes_an_older_one() {
    let f = fixture();
    let g = cid(&f, "food.groceries");
    set_budget(&f.conn, g, "2026-03", 30000).unwrap();
    set_budget(&f.conn, g, "2026-06", 40000).unwrap();
    remove_budget(&f.conn, g, "2026-09").unwrap();
    assert_eq!(
        rows_of(&f, "food.groceries"),
        vec![
            (s("2026-03"), Some(s("2026-05")), 30000),
            (s("2026-06"), Some(s("2026-08")), 40000)
        ]
    );
    assert_eq!(month_budgets(&f, "2026-08").budgets.len(), 1);
    assert!(month_budgets(&f, "2026-09").budgets.is_empty());
    assert!(matches!(
        remove_budget(&f.conn, g, "2026-09"),
        Err(LedgerError::NotFound(_))
    ));
    remove_budget(&f.conn, g, "2026-06").unwrap();
    assert_eq!(
        rows_of(&f, "food.groceries"),
        vec![(s("2026-03"), Some(s("2026-05")), 30000)]
    );
    remove_budget(&f.conn, g, "2026-03").unwrap();
    assert!(rows_of(&f, "food.groceries").is_empty());
    set_budget(&f.conn, g, "2026-12", 100).unwrap();
    assert_eq!(
        rows_of(&f, "food.groceries"),
        vec![(s("2026-12"), None, 100)]
    );
}

#[test]
fn remove_budget_also_drops_rows_scheduled_for_later_months() {
    let f = fixture();
    let g = cid(&f, "food.groceries");
    set_budget(&f.conn, g, "2026-03", 30000).unwrap();
    set_budget(&f.conn, g, "2026-12", 45000).unwrap();
    remove_budget(&f.conn, g, "2026-06").unwrap();
    assert_eq!(
        rows_of(&f, "food.groceries"),
        vec![(s("2026-03"), Some(s("2026-05")), 30000)]
    );
    assert!(month_budgets(&f, "2026-12").budgets.is_empty());
}

#[test]
fn group_and_subcategory_budgets_cannot_overlap_in_either_direction() {
    let f = fixture();
    let food = cid(&f, "food");
    let groceries = cid(&f, "food.groceries");
    set_budget(&f.conn, food, "2026-01", 40000).unwrap();
    assert!(matches!(
        set_budget(&f.conn, groceries, "2026-03", 100),
        Err(LedgerError::Conflict(_))
    ));
    assert!(matches!(
        set_budget(&f.conn, groceries, "2025-01", 100),
        Err(LedgerError::Conflict(_))
    ));
    assert_eq!(count(&f.conn, "SELECT count(*) FROM budgets"), 1);
    remove_budget(&f.conn, food, "2026-01").unwrap();

    set_budget(&f.conn, groceries, "2026-03", 100).unwrap();
    assert!(matches!(
        set_budget(&f.conn, food, "2026-01", 40000),
        Err(LedgerError::Conflict(_))
    ));
    assert!(matches!(
        set_budget(&f.conn, food, "2027-06", 40000),
        Err(LedgerError::Conflict(_))
    ));
    remove_budget(&f.conn, groceries, "2026-05").unwrap();
    assert!(matches!(
        set_budget(&f.conn, food, "2026-04", 40000),
        Err(LedgerError::Conflict(_))
    ));
    set_budget(&f.conn, food, "2026-05", 40000).unwrap();
    // Siblings never conflict.
    set_budget(&f.conn, cid(&f, "transport.fuel"), "2026-01", 1).unwrap();
    set_budget(&f.conn, cid(&f, "transport.taxi"), "2026-01", 1).unwrap();
}

#[test]
fn spent_counts_pending_excludes_transfers_and_reduces_with_refunds() {
    let f = fixture();
    set_budget(&f.conn, cid(&f, "transport.fuel"), "2026-01", 10000).unwrap();
    tx(
        &f,
        f.own_a,
        "2026-10-02",
        -5000,
        "gasolinera",
        Some("transport.fuel"),
    );
    tx(
        &f,
        f.own_a,
        "2026-10-03",
        -3000,
        "gasolinera 2",
        Some("transport.fuel"),
    );
    let pending = tx(
        &f,
        f.own_a,
        "2026-10-04",
        -2000,
        "pendiente",
        Some("transport.fuel"),
    );
    make_pending(&f, pending);
    tx(
        &f,
        f.own_a,
        "2026-10-05",
        1500,
        "devolucion",
        Some("transport.fuel"),
    );
    tx(
        &f,
        f.external,
        "2026-10-06",
        -9999,
        "externa",
        Some("transport.fuel"),
    );
    tx(
        &f,
        f.own_a,
        "2026-09-30",
        -4000,
        "mes anterior",
        Some("transport.fuel"),
    );
    tx(
        &f,
        f.own_a,
        "2026-11-01",
        -4000,
        "mes siguiente",
        Some("transport.fuel"),
    );
    tx_src(
        &f,
        f.own_a,
        "2026-10-07",
        -7000,
        "traspaso",
        Some("transfer.internal"),
        "transfer",
    );
    let m = month_budgets(&f, "2026-10");
    let row = &m.budgets[0];
    assert_eq!(row.spent_cents, 8500);
    assert_eq!(row.remaining_cents, 1500);
    assert_eq!((row.percent_bp, row.status), (8500, "ok"));
    assert_eq!(row.avg3_cents, 1333); // September 4000 / 3
}

#[test]
fn status_thresholds_percentages_totals_and_sorting() {
    let f = fixture();
    let b = |key: &str, amount: i64, spent: i64| {
        set_budget(&f.conn, cid(&f, key), "2026-01", amount).unwrap();
        if spent != 0 {
            tx(&f, f.own_a, "2026-10-10", -spent, "gasto", Some(key));
        }
    };
    b("transport.fuel", 1000, 1001); // over
    b("health.pharmacy", 1000, -300); // refund only: negative spend
    b("utilities.electricity", 10000, 100); // ok
    b("education", 2000, 1000); // group budget: spend on a child counts
    f.conn
        .execute(
            "UPDATE transactions SET category_id = ?1 WHERE description = 'gasto' AND amount_cents = -1000",
            [cid(&f, "education.books")],
        )
        .unwrap();
    tx(
        &f,
        f.own_a,
        "2026-10-11",
        -800,
        "gasto grupo",
        Some("education"),
    );
    b("leisure.travel", 1000, 900); // exactly 90 %: warn
    b("leisure.culture", 1000, 899); // just under: ok

    let m = month_budgets(&f, "2026-10");
    let by_name = |name: &str| m.budgets.iter().find(|r| r.name == name).unwrap();
    let fuel = by_name("Combustible");
    assert_eq!(
        (
            fuel.spent_cents,
            fuel.status,
            fuel.percent_bp,
            fuel.remaining_cents
        ),
        (1001, "over", 10010, -1)
    );
    assert_eq!(
        (fuel.parent_name.as_deref(), fuel.is_group),
        (Some("Transporte"), false)
    );
    let pharmacy = by_name("Farmacia");
    assert_eq!(
        (
            pharmacy.spent_cents,
            pharmacy.percent_bp,
            pharmacy.status,
            pharmacy.remaining_cents
        ),
        (-300, -3000, "ok", 1300)
    );
    assert_eq!(
        (by_name("Viajes").status, by_name("Viajes").percent_bp),
        ("warn", 9000)
    );
    assert_eq!(by_name("Cultura").status, "ok");
    assert_eq!(by_name("Luz").percent_bp, 100);
    let edu = by_name("Educación");
    assert!(edu.is_group && edu.parent_id.is_none());
    assert_eq!(
        (edu.spent_cents, edu.percent_bp, edu.status),
        (1800, 9000, "warn")
    );
    assert_eq!(edu.child_names, vec![s("Cursos"), s("Libros")]);
    let pcts: Vec<i64> = m.budgets.iter().map(|r| r.percent_bp).collect();
    assert!(pcts.windows(2).all(|w| w[0] >= w[1]));
    assert_eq!(m.budgets.first().unwrap().name, "Combustible");
    assert_eq!(m.budgets.last().unwrap().name, "Farmacia");
    assert_eq!(m.totals.count, 6);
    assert_eq!(
        m.totals.budget_cents,
        1000 + 1000 + 10000 + 2000 + 1000 + 1000
    );
    assert_eq!(m.totals.spent_cents, 1001 - 300 + 100 + 1800 + 900 + 899);
    assert_eq!(
        m.totals.remaining_cents,
        m.totals.budget_cents - m.totals.spent_cents
    );
    assert_eq!((m.totals.over_count, m.totals.over_cents), (1, 1));
}

#[test]
fn colours_inherit_from_the_group() {
    let f = fixture();
    let leisure = cid(&f, "leisure");
    let travel = cid(&f, "leisure.travel");
    set_budget(&f.conn, travel, "2026-01", 1000).unwrap();
    assert_eq!(month_budgets(&f, "2026-10").budgets[0].color, None);
    update_category(&f.conn, leisure, "Ocio", None, Some("cat-4")).unwrap();
    assert_eq!(
        month_budgets(&f, "2026-10").budgets[0].color.as_deref(),
        Some("cat-4")
    );
    update_category(&f.conn, travel, "Viajes", Some(leisure), Some("cat-7")).unwrap();
    assert_eq!(
        month_budgets(&f, "2026-10").budgets[0].color.as_deref(),
        Some("cat-7")
    );
    let listed = categories(&f.conn).unwrap();
    assert_eq!(
        listed
            .iter()
            .find(|c| c.id == travel)
            .unwrap()
            .color
            .as_deref(),
        Some("cat-7")
    );
}

#[test]
fn avg3_is_the_net_spend_of_the_three_previous_months_rounded_half_away_from_zero() {
    let f = fixture();
    set_budget(&f.conn, cid(&f, "health.pharmacy"), "2026-01", 5000).unwrap();
    set_budget(&f.conn, cid(&f, "food"), "2026-01", 5000).unwrap();
    // Food group: children and the group itself count; June and October are outside.
    tx(
        &f,
        f.own_a,
        "2026-07-01",
        -1000,
        "x",
        Some("food.groceries"),
    );
    tx(&f, f.own_a, "2026-08-31", -1000, "x", Some("food.cafes"));
    tx(&f, f.own_a, "2026-09-30", 1, "x", Some("food"));
    tx(&f, f.own_a, "2026-06-30", -9999, "fuera", Some("food"));
    tx(&f, f.own_a, "2026-10-01", -9999, "mes actual", Some("food"));
    // Pharmacy: refunds only -> negative average.
    tx(
        &f,
        f.own_a,
        "2026-07-10",
        1000,
        "reembolso",
        Some("health.pharmacy"),
    );
    tx(
        &f,
        f.own_a,
        "2026-08-10",
        1000,
        "reembolso",
        Some("health.pharmacy"),
    );
    let m = month_budgets(&f, "2026-10");
    let avg = |name: &str| {
        m.budgets
            .iter()
            .find(|r| r.name == name)
            .unwrap()
            .avg3_cents
    };
    assert_eq!(avg("Alimentación"), 666); // 1999 / 3 = 666.33
    assert_eq!(avg("Farmacia"), -667); // -2000 / 3 = -666.67
                                       // January looks at Oct-Dec of the previous year.
    tx(
        &f,
        f.own_a,
        "2025-10-05",
        -3000,
        "x",
        Some("food.groceries"),
    );
    tx(
        &f,
        f.own_a,
        "2025-09-30",
        -3000,
        "x",
        Some("food.groceries"),
    );
    let jan = budgets(&f.conn, "2026-02", "2026-10-20").unwrap();
    assert_eq!(
        jan.budgets
            .iter()
            .find(|r| r.name == "Alimentación")
            .unwrap()
            .avg3_cents,
        0
    );
}

#[test]
fn unbudgeted_spend_and_uncategorized_cents() {
    let f = fixture();
    set_budget(&f.conn, cid(&f, "shopping.tech"), "2026-01", 5000).unwrap();
    set_budget(&f.conn, cid(&f, "food"), "2026-01", 5000).unwrap();
    tx(
        &f,
        f.own_a,
        "2026-10-01",
        -700,
        "x",
        Some("leisure.going_out"),
    );
    tx(&f, f.own_a, "2026-10-02", -300, "x", Some("finance.fees"));
    tx(&f, f.own_a, "2026-10-03", -120, "x", Some("shopping.gifts")); // sibling budgeted: still uncovered
    tx(&f, f.own_a, "2026-10-04", -250, "x", Some("shopping")); // group row, a child budgeted: covered
    tx(&f, f.own_a, "2026-10-05", -400, "x", Some("food.cafes")); // covered by the group budget
    tx(&f, f.own_a, "2026-10-06", -500, "x", Some("leisure")); // group's own row, nothing budgeted
    tx(
        &f,
        f.own_a,
        "2026-10-07",
        100_000,
        "nomina",
        Some("income.salary"),
    ); // income never listed
    tx(&f, f.own_a, "2026-10-08", 300, "x", Some("finance.taxes")); // net negative spend: not listed
    tx_src(
        &f,
        f.own_a,
        "2026-10-09",
        -8000,
        "traspaso",
        Some("transfer.internal"),
        "transfer",
    );
    tx(
        &f,
        f.external,
        "2026-10-10",
        -999,
        "externa",
        Some("leisure.going_out"),
    );
    tx(
        &f,
        f.own_a,
        "2026-09-10",
        -600,
        "x",
        Some("leisure.going_out"),
    );
    tx(
        &f,
        f.own_a,
        "2026-08-10",
        -300,
        "x",
        Some("leisure.going_out"),
    );
    // Uncategorized: no category or "Sin categoría", outflows only.
    tx_src(&f, f.own_a, "2026-10-11", -800, "sin", None, "none");
    tx(
        &f,
        f.own_a,
        "2026-10-12",
        -200,
        "sin 2",
        Some("uncategorized"),
    );
    tx_src(
        &f,
        f.own_a,
        "2026-10-13",
        1000,
        "ingreso sin categoria",
        None,
        "none",
    );

    let m = month_budgets(&f, "2026-10");
    let list: Vec<(&str, Option<&str>, i64)> = m
        .unbudgeted
        .iter()
        .map(|u| (u.name.as_str(), u.parent_name.as_deref(), u.spent_cents))
        .collect();
    assert_eq!(
        list,
        vec![
            ("Salidas", Some("Ocio"), 700),
            ("Ocio", None, 500),
            ("Comisiones", Some("Finanzas"), 300),
            ("Regalos", Some("Compras"), 120)
        ]
    );
    assert_eq!(m.unbudgeted[0].avg3_cents, 300); // (600 + 300) / 3
    assert_eq!(m.uncategorized_cents, 1000);
    assert_eq!(m.totals.count, 2);
}

#[test]
fn month_metadata() {
    let f = fixture();
    let m = budgets(&f.conn, "2026-10", "2026-10-20").unwrap();
    assert_eq!(
        (
            m.month.as_str(),
            m.is_current_month,
            m.day_of_month,
            m.days_in_month
        ),
        ("2026-10", true, Some(20), 31)
    );
    let past = budgets(&f.conn, "2026-02", "2026-10-20").unwrap();
    assert_eq!(
        (past.is_current_month, past.day_of_month, past.days_in_month),
        (false, None, 28)
    );
    assert_eq!(
        budgets(&f.conn, "2028-02", "2026-10-20")
            .unwrap()
            .days_in_month,
        29
    );
    assert_eq!((m.totals.count, m.totals.budget_cents), (0, 0));
    assert!(budgets(&f.conn, "2026-1", "2026-10-20").is_err());
    assert!(budgets(&f.conn, "2026-10", "bad").is_err());
}
