---
title: "ADR 0005: Encrypted backups and recovery"
---

# ADR 0005: Encrypted backups and recovery

- **Status:** Accepted (2026-11-16)
- **Date:** 2026-11-16

## Context

The SQLCipher database key lives only in Windows Credential Manager (ADR 0002/0003). Losing that entry, the disk, or the machine means losing all data, including the owner's real bank history. ADR 0002 requires an encrypted backup that carries the key protected by a user passphrase. Today the app also has a dangerous startup path: if the keyring entry is missing, `load_or_create_db_key` creates and stores a new key and then fails to open the existing database.

Decisions confirmed with the owner on 2026-11-16: database + bank config, manual backups with a reminder, a recovery mode at startup, and a zxcvbn-checked passphrase.

## Decision

### What a backup contains

| Included | Not included |
|---|---|
| A consistent copy of the database, still SQLCipher-encrypted with the current database key | The Enable Banking private key (`.pem`) |
| The current database key (32 bytes) | The Enable Banking session id / consent expiry (keyring) |
| `enable-banking.json` if present (application id and bank, not secret) | Logs, temp files, `backup-state.json` |
| A non-secret manifest: format, creation time (UTC), app version, schema version, sizes | |

After restoring on a new machine the owner reconnects the bank (new consent). Carrying the live database key means a backup can also **unlock the current database** if only the keyring entry was lost, recovering data newer than the backup.

### File format (`.plbackup`, version 1)

All integers little-endian. The 72-byte header is authenticated as AEAD associated data.

| Offset | Size | Field |
|---|---|---|
| 0 | 8 | magic `PLEDGBK\0` |
| 8 | 2 | format version = `1` |
| 10 | 1 | KDF id: `1` = Argon2id (v0x13) |
| 11 | 1 | AEAD id: `1` = XChaCha20-Poly1305 |
| 12 | 4 | Argon2 memory cost (KiB) |
| 16 | 4 | Argon2 time cost |
| 20 | 4 | Argon2 parallelism |
| 24 | 16 | salt (random) |
| 40 | 24 | nonce (random) |
| 64 | 8 | ciphertext length (must equal file length − 72) |
| 72 | n | ciphertext (plaintext + 16-byte Poly1305 tag) |

Plaintext: `db_key (32) ‖ manifest_len (u32) ‖ manifest JSON ‖ database bytes ‖ config bytes`. Lengths of the database and config sections are in the manifest and must add up exactly.

- **Passphrase → key:** passphrase normalized to Unicode NFC, then Argon2id with **m = 256 MiB, t = 3, p = 4**, 16-byte random salt, 32-byte output. The parameters are stored per file so they can be raised later without a format change.
- **Encryption:** XChaCha20-Poly1305 with a random 192-bit nonce and a fresh salt (hence a fresh key) per backup. One-shot AEAD over the whole payload; the plaintext is capped at 1 GiB (the database is a few MB).
- **Reading order:** check magic → reject unknown version (`UnsupportedVersion`) → reject unknown ids and parameters outside safe bounds (m 32 MiB–1 GiB, t 1–10, p 1–16) **before** running the KDF, so a crafted file can not make the app allocate unbounded memory → check length → derive → decrypt. A wrong passphrase and a tampered file give the same error (`WrongPassphraseOrCorrupt`); the AEAD can not tell them apart and we do not try.
- **Crates:** RustCrypto `argon2` and `chacha20poly1305` (audited, widely used), `zxcvbn` for strength, existing `zeroize`/`getrandom`. No custom primitives.

### Creating a backup

1. Hold the bank "busy" guard (no sync mid-way) and the database mutex (the app is the single writer, so the copy is consistent).
2. `ATTACH` a temporary file in the data dir with the **same** database key and run `sqlcipher_export`; copy `user_version` explicitly (the export does not). The snapshot is encrypted on disk at all times.
3. Build the plaintext in zeroizing buffers, encrypt, write `<target>.partial`, flush + fsync, rename to the target. Delete the temporary snapshot.
4. **Verify**: read the file back, decrypt, open the embedded database and run `PRAGMA integrity_check`. Only a verified backup updates `backup-state.json` (last backup time and file name; no path components beyond the name, nothing secret).

Backups are manual: "Crear copia" opens a native Save dialog (driven from Rust, no dialog permission in the WebView), defaulting to `Documents\Pocket Ledger\Copias\pocket-ledger-AAAA-MM-DD-HHMM.plbackup`. The app never deletes backups. Ajustes shows a reminder when the last verified backup is older than 7 days or there is none.

### Restoring (app running)

1. Decrypt and parse (as above). Write the embedded database to a staging file in the data dir (still encrypted under the backup's key), open it **without migrating**, run `PRAGMA integrity_check`, and refuse a schema version newer than this build supports (`NewerSchema`).
2. `sqlcipher_export` the staging database into `pocket-ledger.db.restoring` encrypted with the **current** keyring key (keeps a single key per installation), copy `user_version`, then open it with `db::open` (forward migrations) and check it again.
3. Only now touch the live database: take the busy guard and the mutex, checkpoint the WAL, swap the app connection for a placeholder, rename `pocket-ledger.db` → `pocket-ledger.db.pre-restore-<UTC>`, rename the restored file into place and open it. If opening fails, rename back and reopen the original. The pre-restore copy is kept (it is encrypted) so a mistaken restore can be undone by hand.
4. `enable-banking.json` from the backup replaces the current one only after the database swap succeeded; the old one is kept as `.pre-restore-<UTC>`. The keyring bank session is left as is.

Staging and temporary files are removed on every exit path, and stale ones from a crash are removed at the next startup.

### Recovery mode (startup)

| Database file | Keyring key | Result |
|---|---|---|
| missing | any | Normal start; create a key only if none exists |
| present | present and opens it | Normal start |
| present | missing, malformed, or does not open it | **Recovery mode**: no key is created or overwritten, the file is not touched |

In recovery mode only the recovery screen and its commands work; bank commands and the scheduler refuse to run. The owner can:

- **Unlock with a backup:** pick a backup + passphrase; the key inside it is tried against the existing database; if it opens, it is written to the keyring (a previous, wrong entry is first saved as `db-key.replaced-<UTC>`) and the app returns to normal mode with all current data.
- **Restore a backup:** a fresh key is generated in memory and the replacement database is fully built and checked first; only then is any existing entry saved aside as above and the fresh key stored, the unreadable file renamed to `pocket-ledger.db.unreadable-<UTC>` (never deleted), and the replacement moved into place. A failure before the swap leaves the keyring and the file exactly as they were.

### Passphrase handling

- Asked twice on create; never stored, logged, or returned to the UI. Received as a string over IPC and moved into a zeroizing buffer at once.
- Accepted if at least 12 characters **and** zxcvbn score ≥ 3; the UI suggests 4–5 random words.
- The create dialog states clearly that a forgotten passphrase makes the backup unrecoverable and that nobody can reset it.

## Consequences

- A backup file plus its passphrase gives full access to the data as of that backup, and to the database key of that time. The file is protected by Argon2id (memory-hard) and AEAD; strength rests on the passphrase, hence the zxcvbn gate.
- Restore always re-encrypts to the local key, so the keyring holds one key per installation and old backups keep working after a reinstall.
- One-shot AEAD keeps the whole backup in memory (a few MB today). If the database grows past ~1 GiB, a chunked STREAM construction needs a format version 2.
- Scheduled backups would need the passphrase or a public-key design; left out on purpose.
- Bank commands added later must honour recovery mode (`AppState::require_ready`).
