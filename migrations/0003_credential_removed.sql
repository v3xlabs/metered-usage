-- The first credential list that no longer named the credential, cleared when a later list
-- names it again. A credential CLIProxy deleted or replaced keeps its row, so its events and
-- plans stay readable.
ALTER TABLE credential_state ADD COLUMN removed_at TEXT;

-- A merge now moves the plans of the merged account with its events. Plans left on an
-- account merged before that follow it to the account that absorbed it.
UPDATE plan SET account_id = (
    SELECT account.merged_into_account_id FROM account WHERE account.account_id = plan.account_id
)
WHERE account_id IN (SELECT account_id FROM account WHERE merged_into_account_id IS NOT NULL);
