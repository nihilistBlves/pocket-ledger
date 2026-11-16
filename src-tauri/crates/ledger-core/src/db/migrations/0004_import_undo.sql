-- 0004_import_undo: an import batch can be undone (its inserted rows are deleted);
-- the batch row stays as history with the time it was undone (UTC ISO text).
ALTER TABLE import_batches ADD COLUMN undone_at TEXT;
