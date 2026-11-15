-- 0001_initial: base schema.
-- Conventions: STRICT tables; money as INTEGER cents (signed, negative = outflow);
-- dates as ISO TEXT (YYYY-MM-DD); audit timestamps as UTC ISO TEXT.
-- Nullable columns that take part in uniqueness are handled with COALESCE
-- expression indexes, because SQLite treats NULLs as distinct in UNIQUE.

CREATE TABLE accounts (
    id             INTEGER PRIMARY KEY,
    name           TEXT NOT NULL,
    institution    TEXT,
    kind           TEXT NOT NULL DEFAULT 'checking'
                   CHECK (kind IN ('checking', 'savings', 'card', 'cash', 'other')),
    currency       TEXT NOT NULL DEFAULT 'EUR' CHECK (length(currency) = 3),
    -- Stable identity derived from account details (e.g. sha256 of the IBAN). Never the IBAN.
    identity_hash  TEXT UNIQUE,
    display_suffix TEXT,                       -- last 4 characters, for display only
    is_own         INTEGER NOT NULL DEFAULT 1 CHECK (is_own IN (0, 1)),
    archived       INTEGER NOT NULL DEFAULT 0 CHECK (archived IN (0, 1)),
    created_at     TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at     TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
) STRICT;

-- Provider account uids change per session, so they live apart from the stable account.
CREATE TABLE account_links (
    id           INTEGER PRIMARY KEY,
    account_id   INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    provider     TEXT NOT NULL CHECK (provider IN ('enable_banking')),
    external_uid TEXT NOT NULL,
    session_ref  TEXT,
    valid_until  TEXT,
    created_at   TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    UNIQUE (provider, external_uid)
) STRICT;
CREATE INDEX ix_account_links_account ON account_links(account_id);

CREATE TABLE categories (
    id         INTEGER PRIMARY KEY,
    parent_id  INTEGER REFERENCES categories(id) ON DELETE RESTRICT,
    key        TEXT UNIQUE,                    -- stable seed key, e.g. 'food.groceries'
    name       TEXT NOT NULL,
    kind       TEXT NOT NULL CHECK (kind IN ('income', 'expense', 'transfer', 'system')),
    sort_order INTEGER NOT NULL DEFAULT 0,
    is_system  INTEGER NOT NULL DEFAULT 0 CHECK (is_system IN (0, 1)),
    color      TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
) STRICT;
-- UNIQUE(parent_id, name) that also covers top-level rows (parent_id NULL).
CREATE UNIQUE INDEX ux_categories_parent_name ON categories(COALESCE(parent_id, 0), name);
CREATE INDEX ix_categories_parent ON categories(parent_id);

CREATE TABLE import_batches (
    id          INTEGER PRIMARY KEY,
    account_id  INTEGER NOT NULL REFERENCES accounts(id) ON DELETE RESTRICT,
    profile     TEXT NOT NULL,
    file_name   TEXT,
    file_sha256 TEXT,
    imported_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    rows_total  INTEGER NOT NULL DEFAULT 0,
    inserted    INTEGER NOT NULL DEFAULT 0,
    updated     INTEGER NOT NULL DEFAULT 0,
    duplicates  INTEGER NOT NULL DEFAULT 0,
    errors      INTEGER NOT NULL DEFAULT 0
) STRICT;
CREATE INDEX ix_import_batches_account ON import_batches(account_id);

CREATE TABLE sync_runs (
    id          INTEGER PRIMARY KEY,
    account_id  INTEGER REFERENCES accounts(id) ON DELETE SET NULL,
    provider    TEXT NOT NULL,
    started_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    finished_at TEXT,
    status      TEXT NOT NULL DEFAULT 'running' CHECK (status IN ('running', 'ok', 'error')),
    fetched     INTEGER NOT NULL DEFAULT 0,
    inserted    INTEGER NOT NULL DEFAULT 0,
    updated     INTEGER NOT NULL DEFAULT 0,
    error_code  TEXT
) STRICT;
CREATE INDEX ix_sync_runs_account ON sync_runs(account_id);

-- Categorization rules. account_id NULL means "any account"; uniqueness treats NULL as 0
-- through the COALESCE expression index below (account ids start at 1).
CREATE TABLE rules (
    id          INTEGER PRIMARY KEY,
    pattern     TEXT NOT NULL,                 -- already normalized by the application
    field       TEXT NOT NULL DEFAULT 'any' CHECK (field IN ('counterparty', 'description', 'any')),
    match_type  TEXT NOT NULL DEFAULT 'contains' CHECK (match_type IN ('contains', 'equals')),
    account_id  INTEGER REFERENCES accounts(id) ON DELETE CASCADE,
    direction   TEXT NOT NULL DEFAULT 'any' CHECK (direction IN ('any', 'in', 'out')),
    category_id INTEGER NOT NULL REFERENCES categories(id) ON DELETE CASCADE,
    origin      TEXT NOT NULL DEFAULT 'manual' CHECK (origin IN ('manual', 'learned')),
    priority    INTEGER NOT NULL DEFAULT 0,
    hits        INTEGER NOT NULL DEFAULT 0,
    last_hit_at TEXT,
    created_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
) STRICT;
CREATE UNIQUE INDEX ux_rules_identity
    ON rules(pattern, field, match_type, COALESCE(account_id, 0), direction);
CREATE INDEX ix_rules_category ON rules(category_id);

CREATE TABLE transactions (
    id              INTEGER PRIMARY KEY,
    account_id      INTEGER NOT NULL REFERENCES accounts(id) ON DELETE RESTRICT,
    status          TEXT NOT NULL CHECK (status IN ('pending', 'booked')),
    booking_date    TEXT CHECK (booking_date IS NULL
                                OR booking_date GLOB '[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]'),
    value_date      TEXT CHECK (value_date IS NULL
                                OR value_date GLOB '[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]'),
    amount_cents    INTEGER NOT NULL,          -- signed; zero allowed (some banks send 0-fee rows)
    currency        TEXT NOT NULL DEFAULT 'EUR' CHECK (length(currency) = 3),
    description     TEXT NOT NULL DEFAULT '',
    counterparty    TEXT,
    remittance      TEXT,
    entry_reference TEXT,
    fingerprint     TEXT NOT NULL,
    occurrence_index INTEGER NOT NULL DEFAULT 0 CHECK (occurrence_index >= 0),
    source          TEXT NOT NULL CHECK (source IN ('bank', 'import', 'manual')),
    import_batch_id INTEGER REFERENCES import_batches(id) ON DELETE SET NULL,
    sync_run_id     INTEGER REFERENCES sync_runs(id) ON DELETE SET NULL,
    category_id     INTEGER REFERENCES categories(id) ON DELETE SET NULL,
    category_source TEXT NOT NULL DEFAULT 'none'
                    CHECK (category_source IN ('none', 'rule', 'learned', 'manual', 'transfer')),
    rule_id         INTEGER REFERENCES rules(id) ON DELETE SET NULL,
    transfer_peer_id INTEGER REFERENCES transactions(id) ON DELETE SET NULL,
    notes           TEXT,
    raw_hash        TEXT,
    created_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    -- Pending rows may lack a booking date; booked rows must have one.
    CHECK (status = 'pending' OR booking_date IS NOT NULL),
    UNIQUE (account_id, fingerprint, occurrence_index)
) STRICT;
-- Same bank-provided reference twice on one account is the same transaction.
CREATE UNIQUE INDEX ux_transactions_entry_reference
    ON transactions(account_id, entry_reference) WHERE entry_reference IS NOT NULL;
CREATE INDEX ix_transactions_account_booking ON transactions(account_id, booking_date);
CREATE INDEX ix_transactions_booking ON transactions(booking_date);
CREATE INDEX ix_transactions_category ON transactions(category_id);
CREATE INDEX ix_transactions_transfer_peer ON transactions(transfer_peer_id);

CREATE TABLE budgets (
    id           INTEGER PRIMARY KEY,
    category_id  INTEGER NOT NULL REFERENCES categories(id) ON DELETE CASCADE,
    period       TEXT NOT NULL DEFAULT 'monthly' CHECK (period IN ('monthly')),
    amount_cents INTEGER NOT NULL CHECK (amount_cents > 0),
    start_month  TEXT NOT NULL CHECK (start_month GLOB '[0-9][0-9][0-9][0-9]-[0-1][0-9]'),
    end_month    TEXT CHECK (end_month IS NULL
                             OR end_month GLOB '[0-9][0-9][0-9][0-9]-[0-1][0-9]'),
    created_at   TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
) STRICT;
CREATE INDEX ix_budgets_category ON budgets(category_id);

CREATE TABLE recurring (
    id                    INTEGER PRIMARY KEY,
    name                  TEXT NOT NULL,
    account_id            INTEGER REFERENCES accounts(id) ON DELETE CASCADE,
    category_id           INTEGER REFERENCES categories(id) ON DELETE SET NULL,
    match_pattern         TEXT NOT NULL,
    expected_amount_cents INTEGER,
    cadence               TEXT NOT NULL CHECK (cadence IN ('weekly', 'monthly', 'quarterly', 'yearly')),
    next_due              TEXT,
    active                INTEGER NOT NULL DEFAULT 1 CHECK (active IN (0, 1)),
    created_at            TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
) STRICT;

CREATE TABLE tags (
    id   INTEGER PRIMARY KEY,
    name TEXT NOT NULL UNIQUE COLLATE NOCASE
) STRICT;

CREATE TABLE transaction_tags (
    transaction_id INTEGER NOT NULL REFERENCES transactions(id) ON DELETE CASCADE,
    tag_id         INTEGER NOT NULL REFERENCES tags(id) ON DELETE CASCADE,
    PRIMARY KEY (transaction_id, tag_id)
) STRICT;
CREATE INDEX ix_transaction_tags_tag ON transaction_tags(tag_id);

CREATE TABLE goals (
    id           INTEGER PRIMARY KEY,
    name         TEXT NOT NULL,
    target_cents INTEGER NOT NULL CHECK (target_cents > 0),
    target_date  TEXT,
    account_id   INTEGER REFERENCES accounts(id) ON DELETE SET NULL,
    archived     INTEGER NOT NULL DEFAULT 0 CHECK (archived IN (0, 1)),
    created_at   TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
) STRICT;

CREATE TABLE import_profiles (
    id        INTEGER PRIMARY KEY,
    name      TEXT NOT NULL UNIQUE,
    spec_json TEXT NOT NULL,
    builtin   INTEGER NOT NULL DEFAULT 0 CHECK (builtin IN (0, 1))
) STRICT;

-- Keep updated_at fresh. The WHEN clause stops the trigger from re-firing itself.
CREATE TRIGGER trg_accounts_updated AFTER UPDATE ON accounts
FOR EACH ROW WHEN NEW.updated_at = OLD.updated_at
BEGIN
    UPDATE accounts SET updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = NEW.id;
END;

CREATE TRIGGER trg_categories_updated AFTER UPDATE ON categories
FOR EACH ROW WHEN NEW.updated_at = OLD.updated_at
BEGIN
    UPDATE categories SET updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = NEW.id;
END;

CREATE TRIGGER trg_rules_updated AFTER UPDATE ON rules
FOR EACH ROW WHEN NEW.updated_at = OLD.updated_at
BEGIN
    UPDATE rules SET updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = NEW.id;
END;

CREATE TRIGGER trg_transactions_updated AFTER UPDATE ON transactions
FOR EACH ROW WHEN NEW.updated_at = OLD.updated_at
BEGIN
    UPDATE transactions SET updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = NEW.id;
END;
