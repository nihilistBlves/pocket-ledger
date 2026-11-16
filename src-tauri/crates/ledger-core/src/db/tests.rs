use super::*;
use rusqlite::params;
use tempfile::TempDir;

/// Fixed fictitious key for tests only.
fn test_key() -> DbKey {
    DbKey::from_bytes([0x42; KEY_LEN])
}

fn other_key() -> DbKey {
    DbKey::from_bytes([0x24; KEY_LEN])
}

fn mem() -> Connection {
    open_in_memory(&test_key()).expect("open in memory")
}

fn user_version(conn: &Connection) -> i64 {
    conn.pragma_query_value(None, "user_version", |r| r.get(0))
        .expect("user_version")
}

fn count(conn: &Connection, sql: &str) -> i64 {
    conn.query_row(sql, [], |r| r.get(0)).expect("count query")
}

fn add_account(conn: &Connection, name: &str) -> i64 {
    conn.execute("INSERT INTO accounts (name) VALUES (?1)", params![name])
        .expect("insert account");
    conn.last_insert_rowid()
}

fn add_tx(
    conn: &Connection,
    account: i64,
    entry_ref: Option<&str>,
    fingerprint: &str,
    occurrence: i64,
) -> rusqlite::Result<usize> {
    conn.execute(
        "INSERT INTO transactions
            (account_id, status, booking_date, amount_cents, entry_reference,
             fingerprint, occurrence_index, source)
         VALUES (?1, 'booked', '2026-01-15', -1250, ?2, ?3, ?4, 'manual')",
        params![account, entry_ref, fingerprint, occurrence],
    )
}

#[test]
fn migrations_validate() {
    migrations().validate().expect("migrations are valid");
}

#[test]
fn cipher_version_is_present() {
    let conn = mem();
    assert!(!cipher_version(&conn).unwrap().is_empty());
}

#[test]
fn migrations_apply_and_are_idempotent_on_reopen() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("ledger.db");

    let conn = open(&path, &test_key()).unwrap();
    assert_eq!(user_version(&conn), 4);
    let categories = count(&conn, "SELECT count(*) FROM categories");
    drop(conn);

    let conn = open(&path, &test_key()).unwrap();
    assert_eq!(user_version(&conn), 4);
    assert_eq!(count(&conn, "SELECT count(*) FROM categories"), categories);
}

#[test]
fn wrong_key_is_reported() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("ledger.db");
    drop(open(&path, &test_key()).unwrap());

    match open(&path, &other_key()) {
        Err(DbError::WrongKeyOrCorrupt) => {}
        other => panic!("expected WrongKeyOrCorrupt, got {other:?}"),
    }
}

#[test]
fn file_is_encrypted_on_disk() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("ledger.db");
    let marker = "PLAINTEXT-MARKER-Zq81xv";

    let conn = open(&path, &test_key()).unwrap();
    let account = add_account(&conn, marker);
    conn.execute(
        "INSERT INTO transactions
            (account_id, status, booking_date, amount_cents, description,
             fingerprint, source)
         VALUES (?1, 'booked', '2026-01-15', -100, ?2, 'fp', 'manual')",
        params![account, marker],
    )
    .unwrap();
    conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()))
        .unwrap();
    drop(conn);

    let bytes = std::fs::read(&path).unwrap();
    assert!(bytes.len() > 4096);
    assert!(!bytes.starts_with(b"SQLite format 3"));
    assert!(!contains(&bytes, marker.as_bytes()));
    assert!(!contains(&bytes, b"CREATE TABLE"));

    let wal = dir.path().join("ledger.db-wal");
    if wal.exists() {
        let wal_bytes = std::fs::read(&wal).unwrap();
        assert!(!contains(&wal_bytes, marker.as_bytes()));
    }
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

#[test]
fn foreign_keys_are_enforced() {
    let conn = mem();
    assert_eq!(count(&conn, "PRAGMA foreign_keys"), 1);
    assert!(add_tx(&conn, 999, None, "fp", 0).is_err());
}

#[test]
fn entry_reference_is_unique_per_account() {
    let conn = mem();
    let a = add_account(&conn, "A");
    let b = add_account(&conn, "B");

    add_tx(&conn, a, Some("REF1"), "fp1", 0).unwrap();
    assert!(add_tx(&conn, a, Some("REF1"), "fp2", 0).is_err());
    // Same reference on another account is a different transaction.
    add_tx(&conn, b, Some("REF1"), "fp1", 0).unwrap();
}

#[test]
fn null_entry_reference_allowed_many_times() {
    let conn = mem();
    let a = add_account(&conn, "A");
    for i in 0..5 {
        add_tx(&conn, a, None, &format!("fp{i}"), 0).unwrap();
    }
    assert_eq!(count(&conn, "SELECT count(*) FROM transactions"), 5);
}

#[test]
fn fingerprint_and_occurrence_are_unique() {
    let conn = mem();
    let a = add_account(&conn, "A");
    add_tx(&conn, a, None, "same", 0).unwrap();
    assert!(add_tx(&conn, a, None, "same", 0).is_err());
    // Identical same-day rows are told apart by the occurrence index.
    add_tx(&conn, a, None, "same", 1).unwrap();
}

#[test]
fn booked_transaction_requires_booking_date() {
    let conn = mem();
    let a = add_account(&conn, "A");
    let booked_without_date = conn.execute(
        "INSERT INTO transactions (account_id, status, amount_cents, fingerprint, source)
         VALUES (?1, 'booked', 100, 'x', 'bank')",
        params![a],
    );
    assert!(booked_without_date.is_err());
    conn.execute(
        "INSERT INTO transactions (account_id, status, amount_cents, fingerprint, source)
         VALUES (?1, 'pending', 100, 'y', 'bank')",
        params![a],
    )
    .unwrap();
}

#[test]
fn rules_uniqueness_treats_null_account_as_one_scope() {
    let conn = mem();
    let cat: i64 = conn
        .query_row(
            "SELECT id FROM categories WHERE key = 'food.groceries'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let a = add_account(&conn, "A");
    let insert = |account: Option<i64>| {
        conn.execute(
            "INSERT INTO rules (pattern, field, match_type, account_id, direction, category_id)
             VALUES ('mercadona', 'any', 'contains', ?1, 'any', ?2)",
            params![account, cat],
        )
    };
    insert(None).unwrap();
    assert!(insert(None).is_err());
    insert(Some(a)).unwrap();
    assert!(insert(Some(a)).is_err());
}

#[test]
fn top_level_category_names_are_unique() {
    let conn = mem();
    let dup = conn.execute(
        "INSERT INTO categories (name, kind) VALUES ('Ingresos', 'income')",
        [],
    );
    assert!(dup.is_err());
}

#[test]
fn updated_at_trigger_fires() {
    let conn = mem();
    let a = add_account(&conn, "A");
    conn.execute(
        "UPDATE accounts SET created_at = '2000-01-01T00:00:00.000Z',
                             updated_at = '2000-01-01T00:00:00.000Z' WHERE id = ?1",
        params![a],
    )
    .unwrap();
    conn.execute("UPDATE accounts SET name = 'B' WHERE id = ?1", params![a])
        .unwrap();
    let updated: String = conn
        .query_row(
            "SELECT updated_at FROM accounts WHERE id = ?1",
            params![a],
            |r| r.get(0),
        )
        .unwrap();
    assert_ne!(updated, "2000-01-01T00:00:00.000Z");
}

#[test]
fn seed_categories_shape() {
    let conn = mem();
    assert_eq!(
        count(
            &conn,
            "SELECT count(*) FROM categories WHERE parent_id IS NULL"
        ),
        12
    );
    assert_eq!(
        count(
            &conn,
            "SELECT count(*) FROM categories WHERE parent_id IS NOT NULL"
        ),
        34
    );

    let kind_of = |key: &str| -> String {
        conn.query_row(
            "SELECT kind FROM categories WHERE key = ?1",
            params![key],
            |r| r.get(0),
        )
        .unwrap()
    };
    assert_eq!(kind_of("income"), "income");
    assert_eq!(kind_of("income.salary"), "income");
    assert_eq!(kind_of("housing.rent_mortgage"), "expense");
    assert_eq!(kind_of("transfer.internal"), "transfer");
    assert_eq!(kind_of("uncategorized"), "system");

    // Only the two system categories are flagged as such, and they have no children.
    assert_eq!(
        count(&conn, "SELECT count(*) FROM categories WHERE is_system = 1"),
        2
    );
    assert_eq!(
        count(
            &conn,
            "SELECT count(*) FROM categories c JOIN categories p ON c.parent_id = p.id
             WHERE p.is_system = 1"
        ),
        0
    );

    let name: String = conn
        .query_row(
            "SELECT name FROM categories WHERE key = 'utilities.internet_mobile'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(name, "Internet y móvil");

    let children: i64 = conn
        .query_row(
            "SELECT count(*) FROM categories c JOIN categories p ON c.parent_id = p.id
             WHERE p.key = 'transport'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(children, 4);
}

#[test]
fn db_key_debug_is_redacted_and_hex_is_correct() {
    let key = test_key();
    assert_eq!(format!("{key:?}"), "DbKey(**redacted**)");
    assert_eq!(key.to_hex().as_str(), "42".repeat(KEY_LEN));
    let back = DbKey::from_hex(key.to_hex().as_str()).unwrap();
    assert_eq!(back.as_bytes(), key.as_bytes());
    assert!(matches!(DbKey::from_hex("zz"), Err(DbError::InvalidKey)));
    assert!(matches!(DbKey::from_hex("abcd"), Err(DbError::InvalidKey)));
}

#[test]
fn generated_keys_differ() {
    let a = DbKey::generate().unwrap();
    let b = DbKey::generate().unwrap();
    assert_ne!(a.as_bytes(), b.as_bytes());
    assert_ne!(a.as_bytes(), &[0u8; KEY_LEN]);
}

#[test]
fn import_batches_gain_undone_at_in_migration_4() {
    let conn = mem();
    let account = add_account(&conn, "Cuenta demo");
    conn.execute(
        "INSERT INTO import_batches (account_id, profile) VALUES (?1, 'p')",
        [account],
    )
    .unwrap();
    let undone: Option<String> = conn
        .query_row("SELECT undone_at FROM import_batches", [], |r| r.get(0))
        .unwrap();
    assert!(undone.is_none());
}
