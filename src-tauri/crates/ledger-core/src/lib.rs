//! Domain core of Pocket Ledger.
//!
//! Pure logic only (money as integer cents, transfer detection, deduplication,
//! categorization). No I/O, no Tauri, no network, so it stays fully unit-testable.

#[cfg(test)]
mod tests {
    #[test]
    fn crate_builds() {
        assert_eq!(2 + 2, 4);
    }
}
