-- 0003_bank_sync: state of the incremental bank sync.

-- One sync_runs row per execution (account_id stays NULL: a run covers every linked
-- account). Used by the sync policy (min interval, daily cap).
ALTER TABLE sync_runs ADD COLUMN "trigger" TEXT NOT NULL DEFAULT 'manual'
    CHECK ("trigger" IN ('manual', 'auto'));
CREATE INDEX ix_sync_runs_started ON sync_runs(started_at);

-- Where each account's last successful sync ended (dates are YYYY-MM-DD).
CREATE TABLE account_sync_state (
    account_id      INTEGER PRIMARY KEY REFERENCES accounts(id) ON DELETE CASCADE,
    last_synced_to  TEXT,
    last_success_at TEXT
) STRICT;

-- Latest balance reported by the bank (cents, signed). Only the latest is kept.
CREATE TABLE account_balances (
    account_id     INTEGER PRIMARY KEY REFERENCES accounts(id) ON DELETE CASCADE,
    amount_cents   INTEGER NOT NULL,
    currency       TEXT NOT NULL,
    balance_type   TEXT NOT NULL,
    reference_date TEXT,
    fetched_at     TEXT NOT NULL
) STRICT;
