//! Backup tests. Fictitious data, temp dirs and `MemoryStore` only: the real
//! database and the real Credential Manager are never touched.

use std::fs;
use std::path::{Path, PathBuf};

use rusqlite::{params, Connection};
use tempfile::TempDir;

use super::format::KDF_RUNS;
use super::restore::swap_in_with;
use super::*;
use crate::db::{self, DbError, DbKey, KEY_LEN};
use crate::keystore::{MemoryStore, SecretStore, DB_KEY_ENTRY};

const DB_NAME: &str = "pocket-ledger.db";
const MERCHANT: &str = "ZZ-FICTITIOUS-MERCHANT-7731";
const PASS: &str = "caballo bateria grapa correcto lunes";
const FAST: KdfParams = KdfParams::MIN;

fn key_a() -> DbKey {
    DbKey::from_bytes([0x42; KEY_LEN])
}

fn key_b() -> DbKey {
    DbKey::from_bytes([0x24; KEY_LEN])
}

/// Opens a fresh database in `dir` and inserts a few fictitious rows.
fn seed(dir: &Path, key: &DbKey, merchant: &str) -> Connection {
    let conn = db::open(&dir.join(DB_NAME), key).unwrap();
    conn.execute("INSERT INTO accounts (name) VALUES ('Cuenta demo')", [])
        .unwrap();
    let account = conn.last_insert_rowid();
    for (i, cents) in [-1250i64, 80000, -399].into_iter().enumerate() {
        conn.execute(
            "INSERT INTO transactions
                (account_id, status, booking_date, amount_cents, description,
                 fingerprint, occurrence_index, source)
             VALUES (?1, 'booked', '2026-01-15', ?2, ?3, ?4, 0, 'manual')",
            params![account, cents, merchant, format!("fp-{i}")],
        )
        .unwrap();
    }
    conn
}

fn rows(conn: &Connection) -> Vec<(String, i64)> {
    let mut stmt = conn
        .prepare("SELECT description, amount_cents FROM transactions ORDER BY id")
        .unwrap();
    let out = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    out
}

fn user_version(conn: &Connection) -> i64 {
    conn.pragma_query_value(None, "user_version", |r| r.get(0))
        .unwrap()
}

fn names(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    v.sort();
    v
}

fn make_backup(conn: &Connection, key: &DbKey, data_dir: &Path, out_dir: &Path) -> PathBuf {
    let target = out_dir.join("test.plbackup");
    create_backup(
        conn,
        key,
        data_dir,
        Some(br#"{"application_id":"demo"}"#),
        PASS,
        &target,
        FAST,
        "0.0.0-test",
    )
    .unwrap();
    target
}

/// A source DB plus one backup of it.
struct Setup {
    src: TempDir,
    out: TempDir,
    conn: Connection,
    backup: PathBuf,
}

fn setup() -> Setup {
    let src = TempDir::new().unwrap();
    let out = TempDir::new().unwrap();
    let conn = seed(src.path(), &key_a(), MERCHANT);
    let backup = make_backup(&conn, &key_a(), src.path(), out.path());
    Setup {
        src,
        out,
        conn,
        backup,
    }
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty() && hay.windows(needle.len()).any(|w| w == needle)
}

fn assert_no_secrets_in(dir: &Path) {
    let needles: Vec<Vec<u8>> = vec![
        key_a().as_bytes().to_vec(),
        key_a().to_hex().as_bytes().to_vec(),
        key_b().as_bytes().to_vec(),
        key_b().to_hex().as_bytes().to_vec(),
        PASS.as_bytes().to_vec(),
        MERCHANT.as_bytes().to_vec(),
    ];
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if !path.is_file() {
            continue;
        }
        let bytes = fs::read(&path).unwrap();
        for n in &needles {
            assert!(!contains(&bytes, n), "secret found in {path:?}");
        }
    }
}

fn checkpoint_and_close(conn: Connection) {
    conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()))
        .unwrap();
    drop(conn);
}

#[test]
fn round_trip_into_another_installation_with_a_different_key() {
    let s = setup();
    let dec = read_backup(&s.backup, PASS).unwrap();
    assert_eq!(dec.db_key.as_bytes(), key_a().as_bytes());
    assert_eq!(dec.manifest.format, 1);
    assert_eq!(dec.manifest.app_version, "0.0.0-test");
    assert_eq!(dec.manifest.schema_version, db::latest_schema_version());
    assert!(dec.manifest.created_at.ends_with('Z'));
    assert_eq!(
        dec.config.as_deref().map(|c| c.as_slice()),
        Some(&br#"{"application_id":"demo"}"#[..])
    );
    assert_eq!(verify_backup(&s.backup, PASS).unwrap(), dec.manifest);

    // "New machine": a different data dir with its own key and an older database.
    let dst = TempDir::new().unwrap();
    let existing = seed(dst.path(), &key_b(), "OLD-ROW");
    checkpoint_and_close(existing);

    let prepared = prepare_restore(&dec, dst.path(), &key_b(), DB_NAME).unwrap();
    assert!(prepared.path().exists());
    let db_path = dst.path().join(DB_NAME);
    let (conn, pre) = swap_in(prepared, &db_path, &key_b()).unwrap();

    assert_eq!(rows(&conn), rows(&s.conn));
    assert_eq!(user_version(&conn), db::latest_schema_version());
    let pre = pre.expect("pre-restore copy");
    assert!(pre.exists());
    assert!(pre.to_string_lossy().contains(".pre-restore-"));
    assert!(!names(dst.path()).iter().any(|n| n.ends_with(".restoring")));
    drop(conn);

    // Opens with the new key and not with the old one.
    assert!(db::open_unmigrated(&db_path, &key_b()).is_ok());
    assert!(matches!(
        db::open_unmigrated(&db_path, &key_a()),
        Err(DbError::WrongKeyOrCorrupt)
    ));
    // The pre-restore copy is the old database, still under the old key.
    let old = db::open_unmigrated(&pre, &key_b()).unwrap();
    assert_eq!(rows(&old)[0].0, "OLD-ROW");
}

#[test]
fn restore_into_empty_dir_has_no_pre_restore_copy() {
    let s = setup();
    let dec = read_backup(&s.backup, PASS).unwrap();
    let dst = TempDir::new().unwrap();
    let prepared = prepare_restore(&dec, dst.path(), &key_b(), DB_NAME).unwrap();
    let (conn, pre) = swap_in(prepared, &dst.path().join(DB_NAME), &key_b()).unwrap();
    assert!(pre.is_none());
    assert_eq!(rows(&conn), rows(&s.conn));
}

#[test]
fn wrong_passphrase_is_one_opaque_error() {
    let s = setup();
    let err = read_backup(&s.backup, "otra frase completamente distinta").unwrap_err();
    assert!(matches!(err, BackupError::WrongPassphraseOrCorrupt));
}

#[test]
fn tampering_is_detected() {
    let s = setup();
    let original = fs::read(&s.backup).unwrap();
    let probe = |bytes: &[u8]| {
        let p = s.out.path().join("tampered.plbackup");
        fs::write(&p, bytes).unwrap();
        read_backup(&p, PASS).unwrap_err()
    };

    // One flipped ciphertext byte.
    let mut t = original.clone();
    let last = t.len() - 20;
    t[last] ^= 0x01;
    assert!(matches!(probe(&t), BackupError::WrongPassphraseOrCorrupt));

    // Header fields that stay in bounds: caught by the AEAD (salt, nonce, t cost).
    for offset in [24usize, 50, 16] {
        let mut t = original.clone();
        t[offset] ^= if offset == 16 { 0x02 } else { 0x01 };
        assert!(
            matches!(probe(&t), BackupError::WrongPassphraseOrCorrupt),
            "offset {offset}"
        );
    }

    // Truncation.
    assert!(matches!(probe(&original[..40]), BackupError::NotABackup));
    assert!(matches!(
        probe(&original[..original.len() - 10]),
        BackupError::InvalidHeader
    ));
    // Trailing garbage breaks the exact length rule.
    let mut t = original.clone();
    t.push(0);
    assert!(matches!(probe(&t), BackupError::InvalidHeader));
    // Not a backup at all.
    assert!(matches!(
        probe(b"hello, this is definitely not a backup file, not even close at all....."),
        BackupError::NotABackup
    ));
}

#[test]
fn unsupported_version_and_unsafe_params_are_rejected_before_the_kdf() {
    let s = setup();
    let original = fs::read(&s.backup).unwrap();
    let probe = |bytes: &[u8]| {
        let p = s.out.path().join("crafted.plbackup");
        fs::write(&p, bytes).unwrap();
        let before = KDF_RUNS.with(|c| c.get());
        let err = read_backup(&p, PASS).unwrap_err();
        assert_eq!(KDF_RUNS.with(|c| c.get()), before, "KDF must not run");
        err
    };

    let mut t = original.clone();
    t[8..10].copy_from_slice(&2u16.to_le_bytes());
    assert!(matches!(probe(&t), BackupError::UnsupportedVersion(2)));

    for m in [2_000_000u32, 1_048_577, 32_767, 0, u32::MAX] {
        let mut t = original.clone();
        t[12..16].copy_from_slice(&m.to_le_bytes());
        assert!(matches!(probe(&t), BackupError::InvalidHeader), "m = {m}");
    }
    for (offset, value) in [(16usize, 0u32), (16, 11), (20, 0), (20, 17)] {
        let mut t = original.clone();
        t[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        assert!(matches!(probe(&t), BackupError::InvalidHeader));
    }
    // Unknown KDF / AEAD ids.
    for offset in [10usize, 11] {
        let mut t = original.clone();
        t[offset] = 9;
        assert!(matches!(probe(&t), BackupError::InvalidHeader));
    }
    // A wrong ciphertext length.
    let mut t = original.clone();
    t[64..72].copy_from_slice(&u64::MAX.to_le_bytes());
    assert!(matches!(probe(&t), BackupError::InvalidHeader));
}

#[test]
fn failed_swap_is_rolled_back() {
    let s = setup();
    let dec = read_backup(&s.backup, PASS).unwrap();
    let dst = TempDir::new().unwrap();
    let live = seed(dst.path(), &key_b(), "LIVE-ROW");
    let live_rows = rows(&live);
    checkpoint_and_close(live);
    let db_path = dst.path().join(DB_NAME);

    let prepared = prepare_restore(&dec, dst.path(), &key_b(), DB_NAME).unwrap();
    // Open the new file for real, then fail, so the rollback is exercised with
    // the replacement already in place.
    let err = swap_in_with(prepared, &db_path, &key_b(), &|p, k| {
        let _conn = db::open(p, k)?;
        Err(DbError::WrongKeyOrCorrupt)
    })
    .unwrap_err();
    assert!(matches!(err, BackupError::RestoreRolledBack));

    let back = db::open(&db_path, &key_b()).unwrap();
    assert_eq!(rows(&back), live_rows);
    drop(back);
    for n in names(dst.path()) {
        assert!(
            !n.contains("restoring") && !n.contains("pre-restore") && !n.contains("staging"),
            "leftover {n}"
        );
    }
}

#[test]
fn newer_schema_is_refused_and_live_db_untouched() {
    let src = TempDir::new().unwrap();
    let out = TempDir::new().unwrap();
    let conn = seed(src.path(), &key_a(), MERCHANT);
    conn.pragma_update(None, "user_version", db::latest_schema_version() + 1)
        .unwrap();
    let backup = make_backup(&conn, &key_a(), src.path(), out.path());
    let dec = read_backup(&backup, PASS).unwrap();
    assert_eq!(dec.manifest.schema_version, db::latest_schema_version() + 1);

    let dst = TempDir::new().unwrap();
    let live = seed(dst.path(), &key_b(), "LIVE-ROW");
    let live_rows = rows(&live);
    checkpoint_and_close(live);
    let before = fs::read(dst.path().join(DB_NAME)).unwrap();

    let err = prepare_restore(&dec, dst.path(), &key_b(), DB_NAME).unwrap_err();
    assert!(matches!(err, BackupError::NewerSchema));
    assert_eq!(names(dst.path()), vec![DB_NAME.to_string()]);
    assert_eq!(fs::read(dst.path().join(DB_NAME)).unwrap(), before);
    let again = db::open(&dst.path().join(DB_NAME), &key_b()).unwrap();
    assert_eq!(rows(&again), live_rows);
}

#[test]
fn no_secrets_or_plaintext_on_disk_and_none_in_messages() {
    let s = setup();
    assert_no_secrets_in(s.out.path());
    assert_no_secrets_in(s.src.path());
    // No stale temp files after create.
    for n in names(s.src.path()) {
        assert!(
            !n.starts_with(".backup-snapshot-") && !n.ends_with(".partial"),
            "stale {n}"
        );
    }
    assert_eq!(names(s.out.path()), vec!["test.plbackup".to_string()]);

    let dec = read_backup(&s.backup, PASS).unwrap();
    let dst = TempDir::new().unwrap();
    let existing = seed(dst.path(), &key_b(), "OLD-ROW");
    checkpoint_and_close(existing);
    let prepared = prepare_restore(&dec, dst.path(), &key_b(), DB_NAME).unwrap();
    let (conn, _pre) = swap_in(prepared, &dst.path().join(DB_NAME), &key_b()).unwrap();
    checkpoint_and_close(conn);
    assert_no_secrets_in(dst.path());
    assert!(names(dst.path()).iter().all(|n| !n.contains("staging")));

    // Messages.
    let mut texts = vec![format!("{dec:?}")];
    let errors: Vec<BackupError> = vec![
        read_backup(&s.backup, "wrong passphrase entirely").unwrap_err(),
        create_backup(
            &s.conn,
            &key_a(),
            s.src.path(),
            None,
            "abc",
            &s.out.path().join("weak.plbackup"),
            FAST,
            "x",
        )
        .unwrap_err(),
        BackupError::Db(DbError::WrongKeyOrCorrupt),
    ];
    for e in &errors {
        texts.push(format!("{e}"));
        texts.push(format!("{e:?}"));
    }
    texts.push(format!("{:?}", key_a()));
    for t in texts {
        for needle in [
            key_a().to_hex().to_string(),
            key_b().to_hex().to_string(),
            PASS.to_string(),
            "wrong passphrase entirely".to_string(),
            MERCHANT.to_string(),
        ] {
            assert!(!t.contains(&needle), "leak in {t}");
        }
    }
}

#[test]
fn passphrase_gate() {
    assert!(!check_passphrase("corta").ok);
    assert_eq!(
        check_passphrase("corta").feedback,
        Some("Usa al menos 12 caracteres.")
    );
    let weak = check_passphrase("password1234");
    assert!(!weak.ok && weak.feedback.is_some());
    assert!(!check_passphrase("aaaaaaaaaaaaaaaa").ok);
    let strong = check_passphrase(PASS);
    assert!(strong.ok, "score {}", strong.score);
    assert!(strong.feedback.is_none());

    let s = setup();
    let err = create_backup(
        &s.conn,
        &key_a(),
        s.src.path(),
        None,
        "password1234",
        &s.out.path().join("weak.plbackup"),
        FAST,
        "x",
    )
    .unwrap_err();
    assert!(matches!(err, BackupError::WeakPassphrase));
    assert!(!s.out.path().join("weak.plbackup").exists());
}

#[test]
fn passphrase_is_nfc_normalized() {
    let composed = "añil caballo bateria grapa correcto";
    let decomposed = "an\u{303}il caballo bateria grapa correcto";
    assert_ne!(composed, decomposed);
    let src = TempDir::new().unwrap();
    let out = TempDir::new().unwrap();
    let conn = seed(src.path(), &key_a(), MERCHANT);
    let target = out.path().join("nfc.plbackup");
    create_backup(
        &conn,
        &key_a(),
        src.path(),
        None,
        composed,
        &target,
        FAST,
        "t",
    )
    .unwrap();
    let dec = read_backup(&target, decomposed).unwrap();
    assert!(dec.config.is_none());
}

fn db_file_bytes(dir: &Path) -> Vec<u8> {
    fs::read(dir.join(DB_NAME)).unwrap()
}

#[test]
fn startup_check_cases() {
    let dir = TempDir::new().unwrap();
    let db_path = dir.path().join(DB_NAME);

    // Database missing: normal start, key created once and reused.
    let store = MemoryStore::new();
    let StartupOutcome::Ready(k1) = startup_check(&db_path, &store).unwrap() else {
        panic!("expected Ready");
    };
    let StartupOutcome::Ready(k2) = startup_check(&db_path, &store).unwrap() else {
        panic!("expected Ready");
    };
    assert_eq!(k1.as_bytes(), k2.as_bytes());
    assert!(!db_path.exists(), "startup_check must not create the file");

    // A malformed entry is never replaced, even with no database.
    let bad = MemoryStore::new();
    bad.set(DB_KEY_ENTRY, b"short").unwrap();
    assert!(matches!(
        startup_check(&db_path, &bad).unwrap(),
        StartupOutcome::Recovery {
            reason: RecoveryReason::KeyMalformed
        }
    ));

    // Database present.
    checkpoint_and_close(seed(dir.path(), &key_a(), MERCHANT));
    let before = db_file_bytes(dir.path());

    let empty = MemoryStore::new();
    assert!(matches!(
        startup_check(&db_path, &empty).unwrap(),
        StartupOutcome::Recovery {
            reason: RecoveryReason::KeyMissing
        }
    ));
    assert!(empty.entry_names().is_empty(), "nothing may be created");

    let wrong = MemoryStore::new();
    wrong.set(DB_KEY_ENTRY, key_b().as_bytes()).unwrap();
    assert!(matches!(
        startup_check(&db_path, &wrong).unwrap(),
        StartupOutcome::Recovery {
            reason: RecoveryReason::KeyWrong
        }
    ));
    assert_eq!(
        wrong.get(DB_KEY_ENTRY).unwrap().unwrap().as_slice(),
        key_b().as_bytes()
    );

    assert!(matches!(
        startup_check(&db_path, &bad).unwrap(),
        StartupOutcome::Recovery {
            reason: RecoveryReason::KeyMalformed
        }
    ));
    assert_eq!(bad.get(DB_KEY_ENTRY).unwrap().unwrap().as_slice(), b"short");

    let good = MemoryStore::new();
    good.set(DB_KEY_ENTRY, key_a().as_bytes()).unwrap();
    assert!(matches!(
        startup_check(&db_path, &good).unwrap(),
        StartupOutcome::Ready(_)
    ));
    assert_eq!(db_file_bytes(dir.path()), before, "file untouched");
}

#[test]
fn unlock_with_backup_restores_key_and_keeps_the_old_entry() {
    let s = setup();
    let dec = read_backup(&s.backup, PASS).unwrap();
    checkpoint_and_close(s.conn);
    let db_path = s.src.path().join(DB_NAME);

    let store = MemoryStore::new();
    store.set(DB_KEY_ENTRY, key_b().as_bytes()).unwrap();
    let key = unlock_with_backup(&db_path, &dec, &store).unwrap();
    assert_eq!(key.as_bytes(), key_a().as_bytes());
    assert_eq!(
        store.get(DB_KEY_ENTRY).unwrap().unwrap().as_slice(),
        key_a().as_bytes()
    );
    let aside: Vec<String> = store
        .entry_names()
        .into_iter()
        .filter(|n| n.starts_with("db-key.replaced-"))
        .collect();
    assert_eq!(aside.len(), 1);
    assert_eq!(
        store.get(&aside[0]).unwrap().unwrap().as_slice(),
        key_b().as_bytes()
    );
    assert!(matches!(
        startup_check(&db_path, &store).unwrap(),
        StartupOutcome::Ready(_)
    ));

    // Missing entry: just set, nothing to save aside.
    let empty = MemoryStore::new();
    unlock_with_backup(&db_path, &dec, &empty).unwrap();
    assert_eq!(empty.entry_names(), vec![DB_KEY_ENTRY.to_string()]);
}

#[test]
fn unlock_with_a_backup_of_another_installation_is_refused() {
    // The backup carries key_a; the existing database uses key_b.
    let s = setup();
    let dec = read_backup(&s.backup, PASS).unwrap();
    let other = TempDir::new().unwrap();
    checkpoint_and_close(seed(other.path(), &key_b(), "OTHER"));
    let store = MemoryStore::new();
    store.set(DB_KEY_ENTRY, b"short").unwrap();
    let err = unlock_with_backup(&other.path().join(DB_NAME), &dec, &store).unwrap_err();
    assert!(matches!(err, BackupError::BackupKeyDoesNotOpenDatabase));
    assert_eq!(store.entry_names(), vec![DB_KEY_ENTRY.to_string()]);
    assert_eq!(
        store.get(DB_KEY_ENTRY).unwrap().unwrap().as_slice(),
        b"short"
    );
}

#[test]
fn recovery_restore_renames_the_unreadable_file_and_never_deletes_it() {
    let s = setup();
    let dec = read_backup(&s.backup, PASS).unwrap();

    let dir = TempDir::new().unwrap();
    checkpoint_and_close(seed(dir.path(), &key_b(), "UNREADABLE-ROW"));
    let db_path = dir.path().join(DB_NAME);
    let before = fs::read(&db_path).unwrap();

    let store = MemoryStore::new();
    store.set(DB_KEY_ENTRY, key_a().as_bytes()).unwrap(); // a key that does not open it
    let (conn, new_key) = recovery_restore(&db_path, &dec, &store).unwrap();
    assert_ne!(new_key.as_bytes(), key_a().as_bytes());
    assert_eq!(rows(&conn), rows(&s.conn));
    let unreadable: Vec<String> = names(dir.path())
        .into_iter()
        .filter(|n| n.starts_with("pocket-ledger.db.unreadable-"))
        .collect();
    assert_eq!(unreadable.len(), 1);
    assert_eq!(fs::read(dir.path().join(&unreadable[0])).unwrap(), before);
    assert_eq!(
        store.get(DB_KEY_ENTRY).unwrap().unwrap().as_slice(),
        new_key.as_bytes()
    );
    let aside: Vec<String> = store
        .entry_names()
        .into_iter()
        .filter(|n| n.starts_with("db-key.replaced-"))
        .collect();
    assert_eq!(aside.len(), 1);
    assert_eq!(
        store.get(&aside[0]).unwrap().unwrap().as_slice(),
        key_a().as_bytes()
    );
    drop(conn);
    assert!(matches!(
        startup_check(&db_path, &store).unwrap(),
        StartupOutcome::Ready(_)
    ));
}

#[test]
fn failed_recovery_restore_leaves_store_and_file_untouched() {
    let s = setup();
    let mut dec = read_backup(&s.backup, PASS).unwrap();
    // Corrupt the embedded database (first page holds the schema).
    let mid = 100;
    dec.db[mid] ^= 0xFF;

    let dir = TempDir::new().unwrap();
    checkpoint_and_close(seed(dir.path(), &key_b(), "UNREADABLE-ROW"));
    let db_path = dir.path().join(DB_NAME);
    let before = fs::read(&db_path).unwrap();
    let store = MemoryStore::new();
    store.set(DB_KEY_ENTRY, key_a().as_bytes()).unwrap();

    assert!(recovery_restore(&db_path, &dec, &store).is_err());
    assert_eq!(fs::read(&db_path).unwrap(), before);
    assert_eq!(store.entry_names(), vec![DB_KEY_ENTRY.to_string()]);
    assert_eq!(
        store.get(DB_KEY_ENTRY).unwrap().unwrap().as_slice(),
        key_a().as_bytes()
    );
    assert_eq!(names(dir.path()), vec![DB_NAME.to_string()]);
}

#[test]
fn cleanup_stale_removes_only_its_patterns() {
    let dir = TempDir::new().unwrap();
    let stale = [
        ".backup-snapshot-aa.tmp",
        ".restore-staging-bb.tmp",
        "pocket-ledger.db.restoring",
        "pocket-ledger.db.restoring-wal",
        "x.plbackup.partial",
    ];
    let keep = [
        "pocket-ledger.db",
        "pocket-ledger.db-wal",
        "enable-banking.json",
        "backup-state.json",
        "pocket-ledger.db.pre-restore-20260101T000000Z",
        "pocket-ledger.db.unreadable-20260101T000000Z",
        "notes.tmp",
        "snapshot.txt",
    ];
    for n in stale.iter().chain(keep.iter()) {
        fs::write(dir.path().join(n), b"x").unwrap();
    }
    fs::create_dir(dir.path().join("sub.partial")).unwrap();
    assert_eq!(cleanup_stale(dir.path()).unwrap(), stale.len());
    let mut expected: Vec<String> = keep.iter().map(|s| s.to_string()).collect();
    expected.push("sub.partial".to_string());
    expected.sort();
    assert_eq!(names(dir.path()), expected);
}

#[test]
fn install_config_keeps_the_previous_file() {
    let dir = TempDir::new().unwrap();
    assert!(install_config(dir.path(), "enable-banking.json", b"one")
        .unwrap()
        .is_none());
    let kept = install_config(dir.path(), "enable-banking.json", b"two")
        .unwrap()
        .expect("previous kept");
    assert_eq!(fs::read(&kept).unwrap(), b"one");
    assert_eq!(
        fs::read(dir.path().join("enable-banking.json")).unwrap(),
        b"two"
    );
    assert!(names(dir.path()).iter().all(|n| !n.ends_with(".partial")));
    assert!(install_config(dir.path(), "../evil.json", b"x").is_err());
}

#[test]
fn production_parameters_end_to_end() {
    let src = TempDir::new().unwrap();
    let out = TempDir::new().unwrap();
    let conn = seed(src.path(), &key_a(), MERCHANT);
    let target = out.path().join("default.plbackup");
    let info = create_backup(
        &conn,
        &key_a(),
        src.path(),
        None,
        PASS,
        &target,
        KdfParams::DEFAULT,
        "0.0.0-test",
    )
    .unwrap();
    assert_eq!(info.file_name, "default.plbackup");
    assert_eq!(info.size, fs::metadata(&target).unwrap().len());
    let file = fs::read(&target).unwrap();
    assert_eq!(&file[..8], b"PLEDGBK\0");
    assert_eq!(
        u32::from_le_bytes(file[12..16].try_into().unwrap()),
        262_144
    );
    assert_eq!(read_backup(&target, PASS).unwrap().manifest.format, 1);
}
