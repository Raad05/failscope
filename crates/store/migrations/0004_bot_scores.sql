-- Bot-likelihood scoring (M9). Per-transaction facts from the decoder, and
-- the signer's score when the failure was ingested. Nullable: rows from
-- before M9 have none of these.
ALTER TABLE failed_tx
    ADD COLUMN cu_price          BIGINT,
    ADD COLUMN cu_limit_explicit BOOLEAN,
    ADD COLUMN jito_tip          BOOLEAN,
    ADD COLUMN bot_score         SMALLINT CHECK (bot_score BETWEEN 0 AND 100),
    ADD COLUMN bot_class         TEXT CHECK (bot_class IN ('likely_bot', 'unclear', 'likely_human')),
    ADD COLUMN bot_reasons       TEXT[],
    -- Raw features behind the score, kept for re-scoring or model fitting.
    ADD COLUMN bot_features      JSONB;

CREATE INDEX failed_tx_signer_slot ON failed_tx (signer, slot);

-- Which quantity an alert counted: failures, or likely-human signers.
ALTER TABLE alert_events
    ADD COLUMN metric TEXT NOT NULL DEFAULT 'failures'
        CHECK (metric IN ('failures', 'human_signers'));
