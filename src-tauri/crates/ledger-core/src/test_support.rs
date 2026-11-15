//! Shared helpers for the pipeline tests. Fictitious data only.

use rusqlite::{params, Connection};

use crate::db::{open_in_memory, DbKey, KEY_LEN};
use crate::model::{NewTransaction, TxStatus};

/// In-memory database with two own accounts and one that is not the user's.
pub struct Fixture {
    pub conn: Connection,
    pub own_a: i64,
    pub own_b: i64,
    pub external: i64,
}

pub fn fixture() -> Fixture {
    let conn = open_in_memory(&DbKey::from_bytes([0x11; KEY_LEN])).expect("open db");
    let own_a = add_account(&conn, "Cuenta demo", 1);
    let own_b = add_account(&conn, "Ahorro demo", 1);
    let external = add_account(&conn, "Cuenta externa demo", 0);
    Fixture {
        conn,
        own_a,
        own_b,
        external,
    }
}

fn add_account(conn: &Connection, name: &str, is_own: i64) -> i64 {
    conn.execute(
        "INSERT INTO accounts (name, is_own) VALUES (?1, ?2)",
        params![name, is_own],
    )
    .expect("insert account");
    conn.last_insert_rowid()
}

pub fn booked(date: &str, cents: i64, description: &str) -> NewTransaction {
    NewTransaction {
        status: TxStatus::Booked,
        booking_date: Some(date.to_string()),
        value_date: None,
        amount_cents: cents,
        currency: "EUR".to_string(),
        description: description.to_string(),
        counterparty: None,
        remittance: None,
        entry_reference: None,
    }
}

pub fn pending(date: &str, cents: i64, description: &str) -> NewTransaction {
    NewTransaction {
        status: TxStatus::Pending,
        booking_date: None,
        value_date: Some(date.to_string()),
        ..booked(date, cents, description)
    }
}

pub fn with_ref(mut tx: NewTransaction, reference: &str) -> NewTransaction {
    tx.entry_reference = Some(reference.to_string());
    tx
}

pub fn category_id(conn: &Connection, key: &str) -> i64 {
    conn.query_row("SELECT id FROM categories WHERE key = ?1", [key], |r| {
        r.get(0)
    })
    .expect("seeded category")
}

pub fn count(conn: &Connection, sql: &str) -> i64 {
    conn.query_row(sql, [], |r| r.get(0)).expect("count")
}

/// Inserts a booked row directly (bypassing the pipeline) and returns its id.
pub fn raw_booked(conn: &Connection, account: i64, date: &str, cents: i64, text: &str) -> i64 {
    let n = count(conn, "SELECT count(*) FROM transactions");
    conn.execute(
        "INSERT INTO transactions
           (account_id, status, booking_date, amount_cents, description, fingerprint, source)
         VALUES (?1, 'booked', ?2, ?3, ?4, ?5, 'import')",
        params![account, date, cents, text, format!("raw{n}")],
    )
    .expect("insert tx");
    conn.last_insert_rowid()
}

pub fn text_of(conn: &Connection, sql: &str, id: i64) -> Option<String> {
    conn.query_row(sql, [id], |r| r.get(0)).expect("text query")
}

pub fn int_of(conn: &Connection, sql: &str, id: i64) -> Option<i64> {
    conn.query_row(sql, [id], |r| r.get(0)).expect("int query")
}
