//! Accounts created by the user (without a bank connection), so a file can be
//! imported into them. The account has no identity yet: the first import whose file
//! names an IBAN in its preamble adopts it (docs/decisions/phase-5a.md).

use rusqlite::{params, Connection};

use super::{LedgerError, Result};

/// Longest account name, in characters.
pub const MAX_ACCOUNT_NAME_CHARS: usize = 60;
const KINDS: [&str; 5] = ["checking", "savings", "card", "cash", "other"];

/// Creates an own, non-archived EUR account and returns its id. `kind` defaults to
/// `checking`. The name is trimmed (inner whitespace collapsed) and must be 1..=60
/// characters and not used by another active account, ignoring case.
pub fn create_manual_account(conn: &Connection, name: &str, kind: Option<&str>) -> Result<i64> {
    let name = name.split_whitespace().collect::<Vec<_>>().join(" ");
    if name.is_empty() || name.chars().count() > MAX_ACCOUNT_NAME_CHARS {
        return Err(LedgerError::InvalidInput("account name length"));
    }
    let kind = kind.unwrap_or("checking");
    if !KINDS.contains(&kind) {
        return Err(LedgerError::InvalidInput("account kind"));
    }
    let taken: bool = conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM accounts WHERE archived = 0 AND lower(name) = lower(?1))",
        [&name],
        |r| r.get(0),
    )?;
    if taken {
        return Err(LedgerError::Conflict("account name in use"));
    }
    conn.execute(
        "INSERT INTO accounts (name, kind, currency, is_own) VALUES (?1, ?2, 'EUR', 1)",
        params![name, kind],
    )?;
    Ok(conn.last_insert_rowid())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::*;

    #[test]
    fn creates_an_own_account_without_identity() {
        let f = fixture();
        let id = create_manual_account(&f.conn, "  Mi   cuenta  ", None).unwrap();
        let (name, kind, own, hash, currency): (String, String, i64, Option<String>, String) = f
            .conn
            .query_row(
                "SELECT name, kind, is_own, identity_hash, currency FROM accounts WHERE id = ?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .unwrap();
        assert_eq!(
            (name.as_str(), kind.as_str(), own, hash, currency.as_str()),
            ("Mi cuenta", "checking", 1, None, "EUR")
        );
        let card = create_manual_account(&f.conn, "Tarjeta", Some("card")).unwrap();
        assert_ne!(card, id);
    }

    #[test]
    fn rejects_empty_long_duplicate_and_unknown_kind() {
        let f = fixture();
        assert!(matches!(
            create_manual_account(&f.conn, "   ", None),
            Err(LedgerError::InvalidInput(_))
        ));
        let long = "x".repeat(MAX_ACCOUNT_NAME_CHARS + 1);
        assert!(matches!(
            create_manual_account(&f.conn, &long, None),
            Err(LedgerError::InvalidInput(_))
        ));
        assert!(matches!(
            create_manual_account(&f.conn, "Cuenta", Some("loan")),
            Err(LedgerError::InvalidInput(_))
        ));
        // "Cuenta demo" is a fixture account; the comparison ignores case.
        assert!(matches!(
            create_manual_account(&f.conn, "cuenta DEMO", None),
            Err(LedgerError::Conflict(_))
        ));
        // Archived accounts do not reserve their name.
        f.conn
            .execute("UPDATE accounts SET archived = 1 WHERE id = ?1", [f.own_b])
            .unwrap();
        assert!(create_manual_account(&f.conn, "Ahorro demo", None).is_ok());
    }
}
