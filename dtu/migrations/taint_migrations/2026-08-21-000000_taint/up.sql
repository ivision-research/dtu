-- The output of a taint analysis run. This database holds keys into the graph
-- database and should be used with the graph database ATTACHed.

-- Exactly one row, describing the run that produced this file.
CREATE TABLE run_info
(
    -- Analyzer options as JSON
    options         TEXT    NOT NULL,
    -- Schema version of this database. Incremented with breaking changes.
    schema_version  INTEGER NOT NULL,
    -- graph._metadata.built_at as it was when the run started
    graph_built_at  BIGINT  NOT NULL,
    dtu_version     TEXT    NOT NULL,
    -- Unix timestamp, UTC, for the most recent run
    started_at      BIGINT  NOT NULL,
    completed       BOOLEAN NOT NULL DEFAULT false
);

-- Enforces the single row without a column whose only purpose is to say so
CREATE UNIQUE INDEX run_info_single_row ON run_info((1));

-- Every method the analysis ran on, deduplicated.
CREATE TABLE analyzed_methods
(
    id          INTEGER NOT NULL,
    -- graph.methods.id
    method      INTEGER NOT NULL,
    -- Whether the method was asked for (Origin::Direct) rather than reached
    -- through the call graph
    direct      BOOLEAN NOT NULL,
    status      TEXT    NOT NULL DEFAULT 'pending',
    error       TEXT             DEFAULT NULL,
    route_count INTEGER NOT NULL DEFAULT 0,

    PRIMARY KEY (id),
    UNIQUE (method),
    CHECK (status IN ('pending', 'failed', 'done')),
    CHECK ((status = 'failed') = (error IS NOT NULL))
);

-- The call graph chains that led to a method, for the methods that were not
-- asked for directly. This is generally populated when method calls are used
-- as taint sources.
CREATE TABLE call_graph_chains
(
    analyzed_method INTEGER NOT NULL,
    chain           INTEGER NOT NULL,
    idx             INTEGER NOT NULL,
    -- graph.methods.id
    method          INTEGER NOT NULL,

    PRIMARY KEY (analyzed_method, chain, idx),
    FOREIGN KEY (analyzed_method) REFERENCES analyzed_methods (id) ON DELETE CASCADE ON UPDATE CASCADE
) WITHOUT ROWID;

-- A seed where tainted data entered the analyzed method.
CREATE TABLE taint_sources
(
    id              INTEGER NOT NULL,
    analyzed_method INTEGER NOT NULL,
    kind            TEXT    NOT NULL,

    PRIMARY KEY (id),
    FOREIGN KEY (analyzed_method) REFERENCES analyzed_methods (id) ON DELETE CASCADE ON UPDATE CASCADE,
    CHECK (kind IN ('param', 'call', 'field'))
);

CREATE INDEX taint_sources_analyzed_method ON taint_sources(analyzed_method);

-- An input parameter by smali register number: p1 is register 1
CREATE TABLE source_params
(
    source   INTEGER NOT NULL,
    register INTEGER NOT NULL,

    PRIMARY KEY (source),
    FOREIGN KEY (source) REFERENCES taint_sources (id) ON DELETE CASCADE ON UPDATE CASCADE
);

-- The result of a call made inside the analyzed method
CREATE TABLE source_calls
(
    source INTEGER  NOT NULL,
    -- NULL-able because we allow searching without this
    class  TEXT,
    name   TEXT     NOT NULL,
    args   TEXT     NOT NULL,
    -- NULL-able because we allow searching without this
    ret    TEXT,

    PRIMARY KEY (source),
    FOREIGN KEY (source) REFERENCES taint_sources (id) ON DELETE CASCADE ON UPDATE CASCADE
);

-- A field read
CREATE TABLE source_fields
(
    source INTEGER  NOT NULL,
    class  TEXT     NOT NULL,
    name   TEXT     NOT NULL,

    PRIMARY KEY (source),
    FOREIGN KEY (source) REFERENCES taint_sources (id) ON DELETE CASCADE ON UPDATE CASCADE
);

CREATE TABLE routes
(
    id          INTEGER NOT NULL,
    source      INTEGER NOT NULL,
    incomplete  BOOLEAN NOT NULL DEFAULT 0,
    phi_count   INTEGER NOT NULL DEFAULT 0,

    PRIMARY KEY (id),
    FOREIGN KEY (source) REFERENCES taint_sources (id) ON DELETE CASCADE ON UPDATE CASCADE
);

CREATE INDEX routes_source ON routes(source);

-- A field write
CREATE TABLE field_sinks
(
    id      INTEGER NOT NULL,
    class   TEXT    NOT NULL,
    name    TEXT    NOT NULL,
    PRIMARY KEY (id),
    UNIQUE (class, name)
);

-- Instruction called on tainted value
CREATE TABLE instruction_sinks
(
    id          INTEGER NOT NULL,
    instruction TEXT    NOT NULL,
    PRIMARY KEY (id),
    UNIQUE (instruction)
);

-- Call to a method we didn't have in the graph database
CREATE TABLE external_sinks
(
    id          INTEGER NOT NULL,
    class       TEXT    NOT NULL,
    name        TEXT    NOT NULL,
    signature   TEXT    NOT NULL,
    PRIMARY KEY (id),
    UNIQUE (class, name, signature)
);


CREATE TABLE sinks
(
    route    INTEGER NOT NULL,
    idx      INTEGER NOT NULL,
    -- graph.methods.id of the method the sink occurred in
    location INTEGER NOT NULL,
    kind     TEXT    NOT NULL,

    -- The sink_id can be an ID into:
    --
    --  - field_sinks
    --  - external_sinks
    --  - instruction_sinks
    --
    -- This field is NULL for phi, array, and methods
    sink_id INTEGER,
    method_id INTEGER,

    PRIMARY KEY (route, idx),
    FOREIGN KEY (route) REFERENCES routes (id) ON DELETE CASCADE ON UPDATE CASCADE,
    CHECK (kind IN ('call', 'field', 'external', 'instruction', 'phi', 'array'))
) WITHOUT ROWID;

CREATE INDEX sink_search ON sinks(kind, sink_id);
CREATE INDEX sinks_location ON sinks(location);

CREATE TABLE sink_filters (
    id      INTEGER NOT NULL,
    route   INTEGER NOT NULL,
    idx     INTEGER NOT NULL,
    kind    TEXT    NOT NULL,
    class   TEXT,
    name    TEXT,
    args    TEXT,
    ret     TEXT,
    PRIMARY KEY (id),
    CHECK (kind IN ('call', 'field', 'external')),
    UNIQUE(route, idx)
);

-- Add external and field sinks automatically, but we'll need to add the graph
-- ones in a different way. That is handled by a temporary trigger after attaching.
CREATE TRIGGER update_sink_filters_external
AFTER INSERT ON sinks
WHEN new.kind = 'external'
BEGIN
    INSERT INTO sink_filters (route, idx, kind, class, name, args)
    SELECT
        new.route,
        new.idx,
        new.kind,
        external_sinks.class,
        external_sinks.name,
        external_sinks.signature
    FROM external_sinks
    WHERE external_sinks.id = new.sink_id;
END;

CREATE TRIGGER update_sink_filters_field
AFTER INSERT ON sinks
WHEN new.kind = 'field'
BEGIN
    INSERT INTO sink_filters (route, idx, kind, class, name)
    SELECT
        new.route,
        new.idx,
        new.kind,
        field_sinks.class,
        field_sinks.name
    FROM field_sinks
    WHERE field_sinks.id = new.sink_id;
END;

-- Table that is used by the UI to mark some analyzed_methods as hidden
CREATE TABLE _hidden_analyzed_methods
(
    analyzed_method INTEGER NOT NULL,

    PRIMARY KEY (analyzed_method),
    FOREIGN KEY (analyzed_method) REFERENCES analyzed_methods (id) ON DELETE CASCADE ON UPDATE CASCADE
) WITHOUT ROWID;

-- Table that is used by the UI to mark some routes as hidden
CREATE TABLE _hidden_routes
(
    route   INTEGER NOT NULL,
    PRIMARY KEY (route),
    FOREIGN KEY (route) REFERENCES routes (id) ON DELETE CASCADE ON UPDATE CASCADE
) WITHOUT ROWID;
