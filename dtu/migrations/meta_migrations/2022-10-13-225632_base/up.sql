CREATE TABLE key_values (
    id INTEGER NOT NULL,
    key VARCHAR(255) NOT NULL,
    value VARCHAR(255) NOT NULL,
    PRIMARY KEY (id),
    UNIQUE(key)
);

INSERT INTO key_values (id, key, value) VALUES(1, 'app_id', 'c.arve');
INSERT INTO key_values (id, key, value) VALUES(2, 'app_pkg', 'c.arve');

CREATE TABLE app_activities (
    id INTEGER NOT NULL,
    name VARCHAR(255) NOT NULL,
    button_android_id VARCHAR(127) NOT NULL,
    button_text VARCHAR(127) NOT NULL,
    status INTEGER NOT NULL DEFAULT 1,
    PRIMARY KEY (id),
    UNIQUE (name),
    UNIQUE (button_android_id)
);

CREATE TABLE app_permissions (
    id INTEGER NOT NULL,
    permission VARCHAR(255) NOT NULL,
    usable BOOLEAN NOT NULL,
    PRIMARY KEY (id),
    UNIQUE (permission)
);

CREATE TABLE progress (
    step INTEGER NOT NULL,
    completed BOOLEAN NOT NULL,
    PRIMARY KEY (step)
);

INSERT INTO progress (step, completed) VALUES (1, false); -- PullAndDecompile
INSERT INTO progress (step, completed) VALUES (2, false); -- SQLDatabaseSetup
INSERT INTO progress (step, completed) VALUES (3, false); -- EmulatorDiff
INSERT INTO progress (step, completed) VALUES (4, false); -- Neo4jSetup
INSERT INTO progress (step, completed) VALUES (5, false); -- FastAutocallResults
INSERT INTO progress (step, completed) VALUES (6, false); -- AppSetup

CREATE TABLE decompile_status (
    id INTEGER NOT NULL,
    device_path VARCHAR(127) NOT NULL,
    host_path VARCHAR(255),
    decompiled BOOLEAN NOT NULL DEFAULT false,
    decompile_attempts INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (id)
);
