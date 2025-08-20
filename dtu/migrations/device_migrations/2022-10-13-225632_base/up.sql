CREATE TABLE apks
(
    id             INTEGER      NOT NULL,
    app_name       VARCHAR(255) NOT NULL,
    name           VARCHAR(255) NOT NULL,
    is_debuggable  BOOLEAN      NOT NULL,
    exists_in_aosp INTEGER      NOT NULL DEFAULT 0,
    PRIMARY KEY (id),
    UNIQUE(app_name, name)
);


CREATE TABLE system_services
(
    id             INTEGER      NOT NULL,
    exists_in_aosp INTEGER      NOT NULL DEFAULT 0,
    name           VARCHAR(255) NOT NULL,
    iface          VARCHAR(255),
    can_get_binder INTEGER      NOT NULL DEFAULT 0,
    PRIMARY KEY (id),
    UNIQUE (name)
);

CREATE TABLE fuzz_results
(
    id                        INTEGER      NOT NULL,
    service_name              VARCHAR(127) NOT NULL,
    method_name               VARCHAR(255) NOT NULL,
    --method_sig VARCHAR(63) NOT NULL, TODO add this back when fast supports it
    exception_thrown          BOOLEAN      NOT NULL,
    security_exception_thrown BOOLEAN      NOT NULL,
    PRIMARY KEY (id)
);

CREATE TABLE protected_broadcasts
(
    id   INTEGER      NOT NULL,
    name VARCHAR(255) NOT NULL,
    PRIMARY KEY (id),
    UNIQUE (name)
);

CREATE TABLE removed_protected_broadcasts
(
    id   INTEGER      NOT NULL,
    name VARCHAR(255) NOT NULL,
    PRIMARY KEY (id),
    UNIQUE (name)
);

CREATE TABLE receivers
(
    id             INTEGER      NOT NULL,
    exists_in_aosp INTEGER      NOT NULL DEFAULT 0,
    class_name     VARCHAR(255) NOT NULL,
    permission     VARCHAR(255),
    exported       BOOLEAN      NOT NULL,
    enabled        BOOLEAN      NOT NULL,
    pkg            VARCHAR(255) NOT NULL,
    apk_id         INTEGER      NOT NULL,
    PRIMARY KEY (id),
    FOREIGN KEY (apk_id) REFERENCES apks (id) ON DELETE CASCADE ON UPDATE CASCADE
);

CREATE TABLE services
(
    id             INTEGER      NOT NULL,
    exists_in_aosp INTEGER      NOT NULL DEFAULT 0,
    class_name     VARCHAR(255) NOT NULL,
    permission     VARCHAR(255),
    exported       BOOLEAN      NOT NULL,
    enabled        BOOLEAN      NOT NULL,
    pkg            VARCHAR(255) NOT NULL,
    apk_id         INTEGER      NOT NULL,
    PRIMARY KEY (id),
    FOREIGN KEY (apk_id) REFERENCES apks (id) ON DELETE CASCADE ON UPDATE CASCADE
);

CREATE TABLE activities
(
    id             INTEGER      NOT NULL,
    exists_in_aosp INTEGER      NOT NULL DEFAULT 0,
    class_name     VARCHAR(255) NOT NULL,
    permission     VARCHAR(255),
    exported       BOOLEAN      NOT NULL,
    enabled        BOOLEAN      NOT NULL,
    pkg            VARCHAR(255) NOT NULL,
    apk_id         INTEGER      NOT NULL,
    PRIMARY KEY (id),
    FOREIGN KEY (apk_id) REFERENCES apks (id) ON DELETE CASCADE ON UPDATE CASCADE
);

CREATE TABLE providers
(
    id                    INTEGER      NOT NULL,
    exists_in_aosp        INTEGER      NOT NULL DEFAULT 0,
    name                  VARCHAR(255) NOT NULL,
    authorities           VARCHAR(255) NOT NULL,
    permission            VARCHAR(255),
    grant_uri_permissions BOOLEAN      NOT NULL,
    read_permission       VARCHAR(255),
    write_permission      VARCHAR(255),
    exported              BOOLEAN      NOT NULL,
    enabled               BOOLEAN      NOT NULL,
    apk_id                INTEGER      NOT NULL,
    PRIMARY KEY (id),
    FOREIGN KEY (apk_id) REFERENCES apks (id) ON DELETE CASCADE ON UPDATE CASCADE
);

CREATE TABLE system_service_impls
(
    id                INTEGER      NOT NULL,
    system_service_id INTEGER      NOT NULL,
    source            VARCHAR(127) NOT NULL,
    class_name        VARCHAR(511) NOT NULL,
    PRIMARY KEY (id),
    FOREIGN KEY (system_service_id) REFERENCES system_services (id) ON DELETE CASCADE ON UPDATE CASCADE
);

CREATE TABLE system_service_methods
(
    id                INTEGER      NOT NULL,
    exists_in_aosp    INTEGER      NOT NULL DEFAULT 0,
    system_service_id INTEGER      NOT NULL,
    transaction_id    INTEGER      NOT NULL,
    name              VARCHAR(255) NOT NULL,
    signature         VARCHAR(511),
    return_type       VARCHAR(127),
    hash_matches_aosp INTEGER      NOT NULL DEFAULT 0,
    sig_matches_aosp  INTEGER      NOT NULL DEFAULT 0,
    smalisa_hash      VARCHAR(44),
    PRIMARY KEY (id),
    FOREIGN KEY (system_service_id) REFERENCES system_services (id) ON DELETE CASCADE ON UPDATE CASCADE
);

CREATE TABLE permissions
(
    id                          INTEGER      NOT NULL,
    exists_in_aosp              INTEGER      NOT NULL DEFAULT 0,
    name                        VARCHAR(255) NOT NULL,
    device_raw_protection_level VARCHAR(127) NOT NULL,
    aosp_raw_protection_level   VARCHAR(127),
    source_apk_id               INTEGER      NOT NULL,
    PRIMARY KEY (id),
    UNIQUE (name),
    FOREIGN KEY (source_apk_id) REFERENCES apks (id) ON DELETE CASCADE ON UPDATE CASCADE
);
