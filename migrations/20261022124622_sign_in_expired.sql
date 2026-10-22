-- A failed quota read proves expiry only when the provider explicitly says its OAuth
-- access token expired. Listings and unrelated failures cannot prove recovery.
ALTER TABLE credential_state ADD COLUMN sign_in_error TEXT CHECK (sign_in_error = 'expired');
