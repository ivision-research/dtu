DROP TABLE device_properties;
DROP TABLE apk_permissions;


ALTER TABLE apks DROP COLUMN is_priv;
ALTER TABLE apks DROP COLUMN device_path;
