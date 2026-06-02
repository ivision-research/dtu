CREATE TABLE strings
(
    id      INTEGER NOT NULL,
    string  TEXT NOT NULL,
    source  INTEGER NOT NULL,
    -- Note that class is absent here: there is no need to store that data
    -- since we can get it out of the method_strings search by just selecting
    -- all methods for a given class.

    PRIMARY KEY (id),
    FOREIGN KEY (source) REFERENCES sources (id) ON DELETE CASCADE ON UPDATE CASCADE
);

-- We don't need multiple copies of a string for a given source
CREATE UNIQUE INDEX strings_source_str ON strings(source, string);


CREATE TABLE method_strings
(
    string  INTEGER NOT NULL,
    method  INTEGER NOT NULL,
    -- We don't include `source` here because it will never differ from
    -- `method.source` and is trivially available in a join

    PRIMARY KEY (method, string),
    FOREIGN KEY (method) REFERENCES methods (id) ON DELETE CASCADE ON UPDATE CASCADE,
    FOREIGN KEY (string) REFERENCES strings (id) ON DELETE CASCADE ON UPDATE CASCADE
) WITHOUT ROWID;
