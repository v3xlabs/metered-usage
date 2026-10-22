-- A shared email proposes a continuation, never identifies a subscription. Remember the
-- user's decision for these two credentials, even if either is removed and listed again.
CREATE TABLE plan_continuation_dismissal (
    account_low INTEGER NOT NULL REFERENCES account (account_id) ON DELETE CASCADE,
    account_high INTEGER NOT NULL REFERENCES account (account_id) ON DELETE CASCADE,
    PRIMARY KEY (account_low, account_high),
    CHECK (account_low < account_high)
);
