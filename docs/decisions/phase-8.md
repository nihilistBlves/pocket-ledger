---
title: Phase 8 decisions
---

# Phase 8 decisions (Importar screen, backend)

## Privacy

- Only Rust opens the native Open dialog and reads the file. The WebView gets the bare file name and size; it never sends or receives a path. The bytes (cap 20 MB) wait in `AppState::picked_import` with their SHA-256 until `import_commit` (cleared on success) or `import_cancel`. A failed commit keeps the stash so the owner can fix the mapping and retry. Cancelling the dialog keeps the previous pick.
- Row errors are row numbers and kinds only. The preview never returns the file's IBAN. Nothing logs file names or contents.

## Profile detection (`detect_profile`)

A stored profile matches when its format fits the extension and content, its header row is found with every column it names, and at least one row is valid. Order: built-in Openbank, other built-ins, user profiles. The generic CSV built-in is not special: it counts only when `Fecha`, `Concepto` and `Importe` are all in the header. Never auto-detected: positional-only profiles (they would match anything) and Windows-1252 CSV profiles on files that are valid UTF-8 with accents (mojibake).

## Dry run

`preview_import` runs the real ingest pipeline (`ingest_in`) inside a transaction it always rolls back, so counts equal what `import_file` would do, including transfers matched and rule categories. Per-row status (`new` / `duplicate` / `updated`) comes from `IngestReport::dispositions`. The identity outcome (`no_identity`, `match`, `will_adopt`, `mismatch`, `belongs_to_other`, `account_not_found`) uses the same check as the real import.

## Batch bookkeeping and undo (decided with the owner)

- `import_batches` stores `rows_total` (valid plus rejected rows), `inserted`, `updated`, `duplicates`, `errors` (rejected rows). The batch row and the ingest share one transaction. A file with no valid row is refused (`NothingToImport`) and leaves no batch.
- Migration 0004 adds `import_batches.undone_at`.
- `undo_import_batch` deletes exactly the rows with `import_batch_id = batch AND source = 'import'`, in one transaction, and sets `undone_at`. Rows that existed before, and duplicates, were never tagged with the batch. Tags go with the rows (FK cascade). Surviving transfer peers are unlinked like `transfers::unlink_transfer`: only a category that came from the link is cleared, then the rules run again.
- **Adopted-by-sync signal:** when bank sync adopts a reference onto, or rewrites, a row whose `source` is `import`, `dedup::apply` stamps it with the sync run id (`sync_run_id`). Imported rows never carry one otherwise. A batch with any such row is not undoable (`BatchHasSyncedRows`); `can_undo` in the history is false for it, for undone batches and for batches that inserted nothing. Rows adopted before this stamp existed are not detectable.
- Undo does not revert an account identity adopted from the file.

## Manual accounts

`create_manual_account(name, kind?)`: own EUR account, `checking` by default, no identity (the first import whose file names an IBAN adopts it, phase 5a). Name trimmed and whitespace-collapsed, 1..60 characters, unique among non-archived accounts ignoring case.

## Refresh

After commit, undo and account creation the backend emits `bank://synced`, which the ledger screens and the bank overview already listen to.
