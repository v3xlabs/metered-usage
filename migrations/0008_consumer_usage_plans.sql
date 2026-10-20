-- Keep a deleted plan's id in the feed so readers know to refresh plans and leverage.
-- usage.updated has no subject fields: each changed batch records one invalidation.
ALTER TABLE change ADD COLUMN plan_id INTEGER;
