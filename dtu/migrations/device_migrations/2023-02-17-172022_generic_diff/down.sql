ALTER TABLE apks
    ADD COLUMN exists_in_aosp INTEGER NOT NULL DEFAULT 0;
UPDATE apks
SET exists_in_aosp = 1
WHERE id IN (SELECT apk FROM apk_diffs WHERE exists_in_diff = 1 AND diff_source = 1);
UPDATE apks
SET exists_in_aosp = -1
WHERE id IN (SELECT apk FROM apk_diffs WHERE exists_in_diff = 0 AND diff_source = 1);
DROP TABLE apk_diffs;

ALTER TABLE system_services
    ADD COLUMN exists_in_aosp INTEGER NOT NULL DEFAULT 0;
UPDATE system_services
SET exists_in_aosp = 1
WHERE id IN (SELECT system_service FROM system_service_diffs WHERE exists_in_diff = 1 AND diff_source = 1);
UPDATE system_services
SET exists_in_aosp = -1
WHERE id IN (SELECT system_service FROM system_service_diffs WHERE exists_in_diff = 0 AND diff_source = 1);
DROP TABLE system_service_diffs;

ALTER TABLE receivers
    ADD COLUMN exists_in_aosp INTEGER NOT NULL DEFAULT 0;
UPDATE receivers
SET exists_in_aosp = 1
WHERE id IN (SELECT receiver FROM receiver_diffs WHERE exists_in_diff = 1 AND diff_source = 1);
UPDATE receivers
SET exists_in_aosp = -1
WHERE id IN (SELECT receiver FROM receiver_diffs WHERE exists_in_diff = 0 AND diff_source = 1);
DROP TABLE receivers_diffs;

ALTER TABLE services
    ADD COLUMN exists_in_aosp INTEGER NOT NULL DEFAULT 0;
UPDATE services
SET exists_in_aosp = 1
WHERE id IN (SELECT service FROM service_diffs WHERE exists_in_diff = 1 AND diff_source = 1);
UPDATE services
SET exists_in_aosp = -1
WHERE id IN (SELECT service FROM service_diffs WHERE exists_in_diff = 0 AND diff_source = 1);
DROP TABLE services_diff;

ALTER TABLE activities
    ADD COLUMN exists_in_aosp INTEGER NOT NULL DEFAULT 0;
UPDATE activities
SET exists_in_aosp = 1
WHERE id IN (SELECT activity FROM activity_diffs WHERE exists_in_diff = 1 AND diff_source = 1);
UPDATE activities
SET exists_in_aosp = -1
WHERE id IN (SELECT activity FROM activity_diffs WHERE exists_in_diff = 0 AND diff_source = 1);
DROP TABLE activity_diffs;

ALTER TABLE providers
    ADD COLUMN exists_in_aosp INTEGER NOT NULL DEFAULT 0;
UPDATE providers
SET exists_in_aosp = 1
WHERE id IN (SELECT provider FROM provider_diffs WHERE exists_in_diff = 1 AND diff_source = 1);
UPDATE providers
SET exists_in_aosp = -1
WHERE id IN (SELECT provider FROM provider_diffs WHERE exists_in_diff = 0 AND diff_source = 1);
DROP TABLE provider_diffs;

ALTER TABLE system_service_methods
    ADD COLUMN exists_in_aosp INTEGER NOT NULL DEFAULT 0;
ALTER TABLE system_service_methods
    ADD COLUMN hash_matches_aosp INTEGER NOT NULL DEFAULT 0;
ALTER TABLE system_service_methods
    ADD COLUMN sig_matches_aosp INTEGER NOT NULL DEFAULT 0;
UPDATE system_service_methods
SET exists_in_aosp = 1
WHERE id IN (SELECT method FROM system_service_methods_diff WHERE exists_in_diff = 1 AND diff_source = 1);
UPDATE system_service_methods
SET exists_in_aosp = -1
WHERE id IN (SELECT method FROM system_service_methods_diff WHERE exists_in_diff = 0 AND diff_source = 1);
UPDATE system_service_methods
SET hash_matches_aosp = system_service_methods_diff.hash_matches_diff
FROM system_service_methods_diff
WHERE system_service_methods_diff.method = system_service_methods.id;
-- These never made sense anyway
UPDATE system_service_methods
SET sig_matches_diff = 1
WHERE id IN (SELECT method FROM system_service_methods_diff WHERE exists_in_diff = 1 AND diff_source = 1);
UPDATE system_service_methods
SET sig_matches_diff = -1
WHERE id IN (SELECT method FROM system_service_methods_diff WHERE exists_in_diff = 0 AND diff_source = 1);
DROP TABLE system_service_method_diffs;

ALTER TABLE permissions
    ADD COLUMN exists_in_aosp INTEGER NOT NULL DEFAULT 0;
ALTER TABLE permissions
    ADD COLUMN aosp_raw_protection_level VARCHAR(127);
ALTER TABLE permissions
    RENAME COLUMN protection_level TO device_raw_protection_level;
UPDATE permissions
SET exists_in_aosp = 1
WHERE id IN (SELECT permission FROM permission_diffs WHERE exists_in_diff = 1 AND diff_source = 1);
UPDATE permissions
SET exists_in_aosp = -1
WHERE id IN (SELECT permission FROM permission_diffs WHERE exists_in_diff = 0 AND diff_source = 1);
UPDATE permissions
SET aosp_raw_protection_level = diff_protection_level
FROM permission_diffs
WHERE permissions.id = permission_diffs.permission;

CREATE TABLE removed_protected_broadcasts
(
    id   INTEGER      NOT NULL,
    name VARCHAR(255) NOT NULL,
    PRIMARY KEY (id),
    UNIQUE (name)
);

INSERT INTO removed_protected_broadcasts (name)
SELECT unprotected_broadcasts.name
FROM unprotected_broadcasts
WHERE unprotected_broadcasts.diff_source = 1;

DROP TABLE unprotected_broadcasts;

DROP TABLE diff_sources;
