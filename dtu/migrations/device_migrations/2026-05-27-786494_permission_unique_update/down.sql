PRAGMA defer_foreign_keys = ON;

CREATE TABLE permissions_old (
    id                INTEGER       NOT NULL,
    name              VARCHAR(255)  NOT NULL,
    protection_level  VARCHAR(127)  NOT NULL,
    source_apk_id     INTEGER       NOT NULL,
    PRIMARY KEY (id),
    UNIQUE (name),
    FOREIGN KEY (source_apk_id) REFERENCES apks (id) ON DELETE CASCADE ON UPDATE CASCADE
);

-- Keep the lowest-id row per name; drop the rest before they hit the unique constraint.
INSERT INTO permissions_old (id, name, protection_level, source_apk_id)
SELECT id, name, protection_level, source_apk_id
FROM permissions
WHERE id IN (SELECT MIN(id) FROM permissions GROUP BY name);

-- Repoint any permission_diffs rows that referenced the dropped duplicates
-- at the surviving row of the same name. (Skip if you'd rather let the FK
-- cascade-delete them.)
UPDATE permission_diffs
SET permission = (
    SELECT MIN(p.id)
    FROM permissions p
    WHERE p.name = (SELECT p2.name FROM permissions p2 WHERE p2.id = permission_diffs.permission)
)
WHERE permission NOT IN (SELECT id FROM permissions_old);

DROP TABLE permissions;
ALTER TABLE permissions_old RENAME TO permissions;

DROP INDEX IF EXISTS permissions_name;
