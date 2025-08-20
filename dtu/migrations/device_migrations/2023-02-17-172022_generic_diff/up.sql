CREATE TABLE diff_sources
(
    id   INTEGER     NOT NULL,
    name VARCHAR(63) NOT NULL,
    PRIMARY KEY (id),
    UNIQUE (name)
);

INSERT INTO diff_sources (id, name)
VALUES (1, 'emulator');

CREATE TABLE apk_diffs
(
    id             INTEGER NOT NULL,
    apk            INTEGER NOT NULL,
    diff_source    INTEGER NOT NULL,
    exists_in_diff BOOLEAN NOT NULL,
    PRIMARY KEY (id),
    FOREIGN KEY (apk) REFERENCES apks (id) ON DELETE CASCADE ON UPDATE CASCADE,
    FOREIGN KEY (diff_source) REFERENCES diff_sources (id) ON DELETE CASCADE ON UPDATE CASCADE,
    UNIQUE (apk, diff_source)
);

INSERT INTO apk_diffs (apk, diff_source, exists_in_diff)
SELECT apks.id,
       1,
       CASE WHEN apks.exists_in_aosp == 1 THEN 1 ELSE 0 END
FROM apks;

ALTER TABLE apks
    DROP COLUMN exists_in_aosp;


CREATE TABLE system_service_diffs
(
    id             INTEGER NOT NULL,
    system_service INTEGER NOT NULL,
    diff_source    INTEGER NOT NULL,
    exists_in_diff BOOLEAN NOT NULL,
    PRIMARY KEY (id),
    FOREIGN KEY (system_service) REFERENCES system_services (id) ON DELETE CASCADE ON UPDATE CASCADE,
    FOREIGN KEY (diff_source) REFERENCES diff_sources (id) ON DELETE CASCADE ON UPDATE CASCADE,
    UNIQUE (system_service, diff_source)
);

INSERT INTO system_service_diffs (system_service, diff_source, exists_in_diff)
SELECT system_services.id,
       1,
       CASE WHEN system_services.exists_in_aosp == 1 THEN 1 ELSE 0 END
FROM system_services;

ALTER TABLE system_services
    DROP COLUMN exists_in_aosp;

CREATE TABLE receiver_diffs
(
    id                      INTEGER NOT NULL,
    receiver                INTEGER NOT NULL,
    diff_source             INTEGER NOT NULL,
    exists_in_diff          BOOLEAN NOT NULL,
    exported_matches_diff   BOOLEAN NOT NULL,
    permission_matches_diff BOOLEAN NOT NULL,
    diff_permission         VARCHAR(127),
    PRIMARY KEY (id),
    FOREIGN KEY (receiver) REFERENCES receivers (id) ON DELETE CASCADE ON UPDATE CASCADE,
    FOREIGN KEY (diff_source) REFERENCES diff_sources (id) ON DELETE CASCADE ON UPDATE CASCADE,
    UNIQUE (receiver, diff_source)
);

-- Since some of the information isn't in there, we'll be conservative and just
-- say it _doesn't_ match the emulator.

INSERT INTO receiver_diffs (receiver, diff_source, exists_in_diff, exported_matches_diff, permission_matches_diff)
SELECT receivers.id,
       1,
       CASE WHEN receivers.exists_in_aosp == 1 THEN 1 ELSE 0 END,
       0,
       0
FROM receivers;

ALTER TABLE receivers
    DROP COLUMN exists_in_aosp;

CREATE TABLE service_diffs
(
    id                      INTEGER NOT NULL,
    service                 INTEGER NOT NULL,
    diff_source             INTEGER NOT NULL,
    exists_in_diff          BOOLEAN NOT NULL,
    exported_matches_diff   BOOLEAN NOT NULL,
    permission_matches_diff BOOLEAN NOT NULL,
    diff_permission         VARCHAR(127),
    PRIMARY KEY (id),
    FOREIGN KEY (service) REFERENCES services (id) ON DELETE CASCADE ON UPDATE CASCADE,
    FOREIGN KEY (diff_source) REFERENCES diff_sources (id) ON DELETE CASCADE ON UPDATE CASCADE,
    UNIQUE (service, diff_source)
);

-- Since some of the information isn't in there, we'll be conservative and just
-- say it _doesn't_ match the emulator.

INSERT INTO service_diffs (service, diff_source, exists_in_diff, exported_matches_diff, permission_matches_diff)
SELECT services.id,
       1,
       CASE WHEN services.exists_in_aosp == 1 THEN 1 ELSE 0 END,
       0,
       0
FROM services;

ALTER TABLE services
    DROP COLUMN exists_in_aosp;

CREATE TABLE activity_diffs
(
    id                      INTEGER NOT NULL,
    activity                INTEGER NOT NULL,
    diff_source             INTEGER NOT NULL,
    exists_in_diff          BOOLEAN NOT NULL,
    exported_matches_diff   BOOLEAN NOT NULL,
    permission_matches_diff BOOLEAN NOT NULL,
    diff_permission         VARCHAR(127),
    PRIMARY KEY (id),
    FOREIGN KEY (activity) REFERENCES activities (id) ON DELETE CASCADE ON UPDATE CASCADE,
    FOREIGN KEY (diff_source) REFERENCES diff_sources (id) ON DELETE CASCADE ON UPDATE CASCADE,
    UNIQUE (activity, diff_source)
);

-- Since some of the information isn't in there, we'll be conservative and just
-- say it _doesn't_ match the emulator.

INSERT INTO activity_diffs (activity, diff_source, exists_in_diff, exported_matches_diff, permission_matches_diff)
SELECT activities.id,
       1,
       CASE WHEN activities.exists_in_aosp == 1 THEN 1 ELSE 0 END,
       0,
       0
FROM activities;

ALTER TABLE activities
    DROP COLUMN exists_in_aosp;

CREATE TABLE provider_diffs
(
    id                            INTEGER NOT NULL,
    provider                      INTEGER NOT NULL,
    diff_source                   INTEGER NOT NULL,
    exists_in_diff                BOOLEAN NOT NULL,
    exported_matches_diff         BOOLEAN NOT NULL,
    permission_matches_diff       BOOLEAN NOT NULL,
    diff_permission               VARCHAR(127),
    write_permission_matches_diff BOOLEAN NOT NULL,
    diff_write_permission         VARCHAR(127),
    read_permission_matches_diff  BOOLEAN NOT NULL,
    diff_read_permission          VARCHAR(127),
    PRIMARY KEY (id),
    FOREIGN KEY (provider) REFERENCES providers (id) ON DELETE CASCADE ON UPDATE CASCADE,
    FOREIGN KEY (diff_source) REFERENCES diff_sources (id) ON DELETE CASCADE ON UPDATE CASCADE,
    UNIQUE (provider, diff_source)
);

-- Since some of the information isn't in there, we'll be conservative and just
-- say it _doesn't_ match the emulator.

INSERT INTO provider_diffs (provider, diff_source, exists_in_diff, exported_matches_diff, permission_matches_diff,
                            write_permission_matches_diff, read_permission_matches_diff)
SELECT providers.id,
       1,
       CASE WHEN providers.exists_in_aosp == 1 THEN 1 ELSE 0 END,
       0,
       0,
       0,
       0
FROM providers;

ALTER TABLE providers
    DROP COLUMN exists_in_aosp;

CREATE TABLE system_service_method_diffs
(
    id                INTEGER NOT NULL,
    method            INTEGER NOT NULL,
    diff_source       INTEGER NOT NULL,
    exists_in_diff    BOOLEAN NOT NULL,
    hash_matches_diff INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (id),
    FOREIGN KEY (method) REFERENCES system_service_methods (id) ON DELETE CASCADE ON UPDATE CASCADE,
    FOREIGN KEY (diff_source) REFERENCES diff_sources (id) ON DELETE CASCADE ON UPDATE CASCADE,
    UNIQUE (method, diff_source)
);

INSERT INTO system_service_method_diffs (method, diff_source, exists_in_diff, hash_matches_diff)
SELECT system_service_methods.id,
       1,
       CASE WHEN system_service_methods.exists_in_aosp == 1 THEN 1 ELSE 0 END,
       system_service_methods.hash_matches_aosp
FROM system_service_methods;

ALTER TABLE system_service_methods
    DROP COLUMN exists_in_aosp;
ALTER TABLE system_service_methods
    DROP COLUMN hash_matches_aosp;
ALTER TABLE system_service_methods
    DROP COLUMN sig_matches_aosp;

CREATE TABLE permission_diffs
(
    id                            INTEGER NOT NULL,
    permission                    INTEGER NOT NULL,
    diff_source                   INTEGER NOT NULL,
    exists_in_diff                BOOLEAN NOT NULL,
    protection_level_matches_diff BOOLEAN NOT NULL,
    diff_protection_level         VARCHAR(127),
    PRIMARY KEY (id),
    FOREIGN KEY (permission) REFERENCES permissions (id) ON DELETE CASCADE ON UPDATE CASCADE,
    FOREIGN KEY (diff_source) REFERENCES diff_sources (id) ON DELETE CASCADE ON UPDATE CASCADE,
    UNIQUE (permission, diff_source)
);

INSERT INTO permission_diffs (permission, diff_source, exists_in_diff, protection_level_matches_diff,
                              diff_protection_level)
SELECT permissions.id,
       1,
       CASE WHEN permissions.exists_in_aosp == 1 THEN 1 ELSE 0 END,
       CASE WHEN permissions.device_raw_protection_level == permissions.aosp_raw_protection_level THEN 1 ELSE 0 END,
       CASE WHEN permissions.aosp_raw_protection_level IS NULL THEN '?' ELSE permissions.aosp_raw_protection_level END
FROM permissions;

ALTER TABLE permissions
    DROP COLUMN exists_in_aosp;
ALTER TABLE permissions
    DROP COLUMN aosp_raw_protection_level;
ALTER TABLE permissions
    RENAME COLUMN device_raw_protection_level TO protection_level;


-- As opposed to the protected_broadcasts table, this table will contain a
-- list of all broadcasts that were protected in the diff but not in the
-- device

CREATE TABLE unprotected_broadcasts
(
    id          INTEGER      NOT NULL,
    name        VARCHAR(255) NOT NULL,
    diff_source INTEGER      NOT NULL,
    PRIMARY KEY (id),
    FOREIGN KEY (diff_source) REFERENCES diff_sources (id) ON DELETE CASCADE ON UPDATE CASCADE,
    UNIQUE (name, diff_source)
);

INSERT INTO unprotected_broadcasts (name, diff_source)
SELECT removed_protected_broadcasts.name, 1
FROM removed_protected_broadcasts;

DROP TABLE removed_protected_broadcasts;