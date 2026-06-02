CREATE TABLE class_fields
(
    id              INTEGER NOT NULL,
    class           INTEGER NOT NULL,
    name            TEXT NOT NULL,
    ty              TEXT NOT NULL,
    -- Default to public
    access_flags    BIGINT  NOT NULL DEFAULT 2,
    -- Source is not included because the class source will be the source this
    -- was discovered in

    PRIMARY KEY (id),
    FOREIGN KEY (class) REFERENCES classes (id) ON DELETE CASCADE ON UPDATE CASCADE
);

CREATE TABLE method_field_access
(
    field   INTEGER NOT NULL,
    method  INTEGER NOT NULL,
    action  INTEGER NOT NULL,
    -- Source is not included because the method source will be the source this
    -- was discovered in


    PRIMARY KEY (field, method, action),
    FOREIGN KEY (field) REFERENCES class_fields (id) ON DELETE CASCADE ON UPDATE CASCADE,
    FOREIGN KEY (method) REFERENCES methods (id) ON DELETE CASCADE ON UPDATE CASCADE
) WITHOUT ROWID;
