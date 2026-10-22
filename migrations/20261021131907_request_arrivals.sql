-- Request arrivals share the durable feed cursor and retention. Other kinds omit failed.
ALTER TABLE change ADD COLUMN failed INTEGER CHECK (failed IN (0, 1));
