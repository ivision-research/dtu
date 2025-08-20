CREATE TABLE device_properties
(
    id      INTEGER         NOT NULL,
    name    VARCHAR(127)    NOT NULL,
    value   VARCHAR(255)    NOT NULL,
    PRIMARY KEY (id),
    UNIQUE (name)
);

CREATE TABLE apk_permissions
(
    id      INTEGER         NOT NULL,
    name    VARCHAR(255)    NOT NULL,
    apk_id  INTEGER         NOT NULL,
    PRIMARY KEY (id),
    UNIQUE (name, apk_id),
    FOREIGN KEY (apk_id) REFERENCES apks (id) ON DELETE CASCADE ON UPDATE CASCADE
);


ALTER TABLE apks ADD COLUMN is_priv BOOLEAN NOT NULL DEFAULT false;
ALTER TABLE apks ADD COLUMN device_path VARCHAR(255) NOT NULL DEFAULT "";
