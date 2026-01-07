-- Note this file does not incldue all indices, but indices will be added
-- later. The rationale for that is that there are typically millions of rows
-- and this database is set up in a single action. Adding some of the indices
-- after loading all the data is faster than having them while inserting. The
-- ones that are in here are used during the loading process and are required.

-- Sources are where we found the item, this is generally "framework" for
-- the entire framework and framework APK, and then the APK name for APKs
CREATE TABLE sources
(
    id      INTEGER NOT NULL,
    name    TEXT    NOT NULL UNIQUE,

    PRIMARY KEY (id)
);

-- Always add the framework source as id = 1 so we can depend on it being that
-- value in the code. This saves us from having to SELECT the framework every
-- time we want its id.
INSERT INTO sources(id, name) VALUES (1, 'framework');

CREATE TABLE classes
(
    id              INTEGER NOT NULL,
    name            TEXT    NOT NULL,
    -- Default to public
    access_flags    BIGINT  NOT NULL DEFAULT 2,
    source          INTEGER NOT NULL,

    PRIMARY KEY (id),
    FOREIGN KEY (source) REFERENCES sources (id) ON DELETE CASCADE ON UPDATE CASCADE
);

-- We look up classes pretty often while loading methods, so this is helpful.
CREATE UNIQUE INDEX class_name_source ON classes(name, source);

CREATE TABLE methods
(
    id              INTEGER NOT NULL,
    class           INTEGER NOT NULL,
    name            TEXT    NOT NULL,
    args            TEXT    NOT NULL,
    ret             TEXT    NOT NULL,
    access_flags    BIGINT  NOT NULL DEFAULT 2,
    source          INTEGER NOT NULL,

    PRIMARY KEY (id),
    FOREIGN KEY (class) REFERENCES classes (id) ON DELETE CASCADE ON UPDATE CASCADE,
    FOREIGN KEY (source) REFERENCES sources (id) ON DELETE CASCADE ON UPDATE CASCADE
);

-- This is used while loading calls
-- Note: this index is _not_ unique. It is possible to have a method generated that
-- has the same (class, name, args) but a different return value.
CREATE INDEX methods_class_name_args_source ON methods(class, name, args, source);

-- In the following tables, source is defined as the source in which the
-- relation was discovered.

CREATE TABLE supers
(
    parent  INTEGER NOT NULL,
    child   INTEGER NOT NULL,
    source  INTEGER NOT NULL,

    FOREIGN KEY (parent) REFERENCES classes (id) ON DELETE CASCADE ON UPDATE CASCADE,
    FOREIGN KEY (child) REFERENCES classes (id) ON DELETE CASCADE ON UPDATE CASCADE,
    FOREIGN KEY (source) REFERENCES sources (id) ON DELETE CASCADE ON UPDATE CASCADE
);

CREATE TABLE interfaces
(
    interface   INTEGER NOT NULL,
    class       INTEGER NOT NULL,
    source      INTEGER NOT NULL,

    FOREIGN KEY (interface) REFERENCES classes (id) ON DELETE CASCADE ON UPDATE CASCADE,
    FOREIGN KEY (class) REFERENCES classes (id) ON DELETE CASCADE ON UPDATE CASCADE,
    FOREIGN KEY (source) REFERENCES sources (id) ON DELETE CASCADE ON UPDATE CASCADE
);

CREATE TABLE calls
(
    caller     INTEGER NOT NULL,
    callee     INTEGER NOT NULL,
    source     INTEGER NOT NULL, 

    FOREIGN KEY (caller) REFERENCES methods (id) ON DELETE CASCADE ON UPDATE CASCADE,
    FOREIGN KEY (callee) REFERENCES methods (id) ON DELETE CASCADE ON UPDATE CASCADE,
    FOREIGN KEY (source) REFERENCES sources (id) ON DELETE CASCADE ON UPDATE CASCADE
);

CREATE TABLE _load_status
(
    source      INTEGER NOT NULL,
    kind        INTEGER NOT NULL,
    FOREIGN KEY (source) REFERENCES sources (id) ON DELETE CASCADE ON UPDATE CASCADE
);

CREATE INDEX _load_status_source ON _load_status(source);
