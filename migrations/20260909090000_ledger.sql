-- Issue #8 Economy ledger baseline: double-entry accounts/transactions/postings.
-- Forward-only: never edit after it has run anywhere; supersede with a new file.
--
-- Shape: single default currency ('CREDITS'). `ledger_accounts` holds one
-- row per user wallet (`kind='user'`, `owner_user_id` NOT NULL) plus a single
-- system/treasury row (`kind='system'`, `owner_user_id` NULL). Balances are
-- always derived (`SUM(ledger_postings.amount)`); no cached balance column.
-- Idempotency: `ledger_transactions.id` is the client-supplied idempotency
-- key (PRIMARY KEY); a duplicate id never creates a second set of postings.
-- Invariant: every committed transaction satisfies `sum(postings) = 0`,
-- enforced by the application inside an explicit transaction AND by the
-- deferred constraint trigger below (fires at COMMIT, so intermediate
-- per-row states inside one transaction do not trip it).

CREATE TABLE ledger_accounts (
    id UUID PRIMARY KEY,
    owner_user_id UUID REFERENCES users (id) ON DELETE CASCADE,
    kind TEXT NOT NULL CHECK (kind IN ('user', 'system')),
    currency TEXT NOT NULL DEFAULT 'CREDITS' CHECK (currency = 'CREDITS'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT ledger_accounts_owner_kind CHECK (
        (kind = 'user' AND owner_user_id IS NOT NULL)
        OR (kind = 'system' AND owner_user_id IS NULL)
    )
);

-- Exactly one wallet per user; exactly one treasury row (single currency).
CREATE UNIQUE INDEX ledger_accounts_owner_unique
    ON ledger_accounts (owner_user_id) WHERE owner_user_id IS NOT NULL;
CREATE UNIQUE INDEX ledger_accounts_system_singleton
    ON ledger_accounts (currency) WHERE kind = 'system';

CREATE TABLE ledger_transactions (
    id UUID PRIMARY KEY,
    kind TEXT NOT NULL DEFAULT 'transfer' CHECK (kind IN ('transfer', 'mint')),
    memo TEXT NOT NULL DEFAULT '',
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE ledger_postings (
    id UUID PRIMARY KEY,
    transaction_id UUID NOT NULL REFERENCES ledger_transactions (id) ON DELETE CASCADE,
    account_id UUID NOT NULL REFERENCES ledger_accounts (id) ON DELETE RESTRICT,
    amount BIGINT NOT NULL CHECK (amount <> 0),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (transaction_id, account_id)
);
CREATE INDEX ledger_postings_account_idx ON ledger_postings (account_id);
CREATE INDEX ledger_postings_transaction_idx ON ledger_postings (transaction_id);

-- Deferred balance guard: at COMMIT every touched transaction must net to 0
-- and keep exactly its two legs. A future multi-leg transaction kind must
-- update this trigger alongside its application code.
CREATE OR REPLACE FUNCTION ledger_assert_balanced()
RETURNS TRIGGER AS $$
DECLARE
    target UUID;
    total BIGINT;
    legs BIGINT;
BEGIN
    IF TG_OP = 'DELETE' THEN
        target := OLD.transaction_id;
    ELSE
        target := NEW.transaction_id;
    END IF;
    SELECT COALESCE(SUM(amount), 0), COUNT(*) INTO total, legs
    FROM ledger_postings WHERE transaction_id = target;
    IF total <> 0 THEN
        RAISE EXCEPTION 'ledger transaction % is unbalanced (sum=%)', target, total
            USING ERRCODE = 'check_violation';
    END IF;
    -- Every baseline transaction carries exactly two legs; a transaction left
    -- with zero legs (e.g. a full delete via direct SQL) is not a valid
    -- ledger state either. There is no API that deletes transactions.
    IF legs <> 2 THEN
        RAISE EXCEPTION 'ledger transaction % must keep both postings (legs=%)', target, legs
            USING ERRCODE = 'check_violation';
    END IF;
    IF TG_OP = 'DELETE' THEN
        RETURN OLD;
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE CONSTRAINT TRIGGER ledger_postings_balanced
    AFTER INSERT OR UPDATE OR DELETE ON ledger_postings
    DEFERRABLE INITIALLY DEFERRED
    FOR EACH ROW EXECUTE FUNCTION ledger_assert_balanced();
