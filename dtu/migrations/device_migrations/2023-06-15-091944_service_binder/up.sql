-- Unknown bool defaulting to unknown
ALTER TABLE services ADD COLUMN returns_binder INTEGER NOT NULL DEFAULT 0;
