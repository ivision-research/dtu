PRAGMA defer_foreign_keys = ON;

CREATE TABLE permissions_new (
    id                INTEGER       NOT NULL,
    name              VARCHAR(255)  NOT NULL,
    protection_level  VARCHAR(127)  NOT NULL,
    source_apk_id     INTEGER       NOT NULL,
    PRIMARY KEY (id),
    UNIQUE (name, source_apk_id),
    FOREIGN KEY (source_apk_id) REFERENCES apks (id) ON DELETE CASCADE ON UPDATE CASCADE
);

INSERT INTO permissions_new (id, name, protection_level, source_apk_id)
SELECT id, name, protection_level, source_apk_id FROM permissions;

DROP TABLE permissions;
ALTER TABLE permissions_new RENAME TO permissions;

CREATE INDEX permissions_name ON permissions(name);
