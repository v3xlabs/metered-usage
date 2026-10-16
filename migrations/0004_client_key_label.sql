-- CLIProxy sent the client's own key as the source of an OAuth credential with no email, and
-- it was kept as the account label. Such a label ends in the masked caller of the events
-- charged to the account. The next credential list names the account again.
UPDATE account SET label = NULL
WHERE auth_kind = 'oauth'
    AND instr(label, '@') = 0
    AND EXISTS (
        SELECT 1 FROM usage_event
        WHERE usage_event.account_id = account.account_id
            AND usage_event.caller = '…' || substr(account.label, -4)
    );
