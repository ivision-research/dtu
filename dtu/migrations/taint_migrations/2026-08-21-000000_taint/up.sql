-- The output of a taint analysis run. This database holds keys into the graph
-- database and should be used with the graph database ATTACHed.
--
-- This is a graph database organized as follows:
-- 
-- All methods that we analyze are placed in `analyzed_methods`. This table is
-- populated immediately upon a run and can be used to resume runs or examine
-- errors. This is generally where everything starts. Each analyzed method has
-- multiple potential taint paths that spread out as a graph. The start of
-- those graphs are available via the `graphs` table. Each graph is represented
-- by `nodes` and `edges`: `nodes` are nodes in the graph and they represent
-- locations that tainted values reached and `edges` are directed edges between
-- nodes.

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

-- Every method the analysis ran on, deduplicated.
CREATE TABLE analyzed_methods
(
    id          INTEGER NOT NULL,
    -- graph.methods.id
    method      INTEGER NOT NULL,
    -- Whether the method was asked for directly rather than reached through
    -- the call graph (seeding)
    direct      BOOLEAN NOT NULL,
    status      TEXT    NOT NULL DEFAULT 'pending',
    error       TEXT             DEFAULT NULL,

    PRIMARY KEY (id),
    UNIQUE (method),
    CHECK (status IN ('pending', 'failed', 'done')),
    CHECK ((status = 'failed') = (error IS NOT NULL))
);

-- A seed where tainted data entered the analyzed method.
CREATE TABLE taint_sources
(
    id              INTEGER NOT NULL,
    kind            TEXT    NOT NULL,
    -- An ID into either source_params/source_calls/source_fields
    source_id       INTEGER NOT NULL,

    PRIMARY KEY (id),
    CHECK (kind IN ('param', 'call', 'field'))
);

CREATE TABLE graphs
(
    id              INTEGER NOT NULL,
    analyzed_method INTEGER NOT NULL,
    entry_node      INTEGER NOT NULL,
    source          INTEGER NOT NULL,

    PRIMARY KEY (id),
    FOREIGN KEY (analyzed_method) REFERENCES analyzed_methods(id) ON DELETE CASCADE ON UPDATE CASCADE,
    FOREIGN KEY (source) REFERENCES taint_sources(id) ON DELETE CASCADE ON UPDATE CASCADE,
    FOREIGN KEY (entry_node) REFERENCES nodes(id) ON DELETE CASCADE ON UPDATE CASCADE
);

CREATE INDEX graph_node ON graphs(entry_node);

CREATE TABLE graph_metadata
(
    graph               INTEGER NOT NULL,
    analyzed_method     INTEGER NOT NULL,
    nphi                INTEGER NOT NULL,
    depth               INTEGER NOT NULL,
    size                INTEGER NOT NULL,

    PRIMARY KEY (graph),

    FOREIGN KEY (graph) REFERENCES graphs(id) ON DELETE CASCADE ON UPDATE CASCADE,
    FOREIGN KEY (analyzed_method) REFERENCES analyzed_methods(id) ON DELETE CASCADE ON UPDATE CASCADE

) WITHOUT ROWID;

CREATE INDEX graph_metadata_analyzed_method ON graph_metadata(analyzed_method);

-- A projection of each graph for all reachable nodes. This is not going to be
-- a complete tree view, but is useful for getting some information about the
-- graph without worrying about cycles.
CREATE TABLE reachable_nodes (
    graph       INTEGER NOT NULL,
    node        INTEGER NOT NULL,
    -- NULL for the entry point
    parent      INTEGER,
    depth       INTEGER NOT NULL,
    -- graph.methods.id of the method this node sits in, taken from the parent edge, or from the
    -- analyzed method itself for the entry node
    location    INTEGER NOT NULL,

    PRIMARY KEY (graph, node),
    FOREIGN KEY (node) REFERENCES nodes(id),
    -- Ensures that there can't be a parent from another graph
    FOREIGN KEY (graph, parent) REFERENCES reachable_nodes(graph, node),
    FOREIGN KEY (graph) REFERENCES graphs(id) ON DELETE CASCADE ON UPDATE CASCADE,

    CHECK ((parent IS NULL AND depth = 0) OR (parent IS NOT NULL AND depth > 0))

) WITHOUT ROWID;

CREATE INDEX reachable_nodes_node ON reachable_nodes(node);

CREATE TABLE nodes
(
    id          INTEGER NOT NULL,
    kind        TEXT NOT NULL,
    -- The sink_id can be an ID into:
    --
    --  - external_field_sinks
    --  - external_call_sinks
    --  - instruction_sinks
    --
    -- This field is NULL for phi, array, fields, and methods
    sink_id     INTEGER,
    -- This is either graph.methods.id or graph.class_fields.id
    graph_id    INTEGER,

    -- This is NULL-able because it is only used for method calls (call/external sinks).
    --
    -- We attach this data to node so we can handle a case like:
    --
    -- A -> K(p0,..) -> ARRAY -> PHI -> J(.., p1) -> Q(p0,..)
    -- B Q(.., p1) -> Z(.., p3) -> J(p0, ..) -> M(.., p2)
    --
    -- Where J is something like J(String, String, int). Calling J(tainted, not_tainted, not_tainted)
    -- is not the same as calling J(not_tainted, tainted, not_tainted) and we need to maintain that
    -- information otherwise it could look like B ends up calling Q with tainted data when it doesn't.
    regs    TEXT NOT NULL,


    PRIMARY KEY(id),

    CHECK (kind IN ('call', 'field', 'ext-field', 'ext-call', 'instr', 'phi', 'array'))
);

CREATE INDEX node_sink_id ON nodes(sink_id);
CREATE INDEX node_graph_id ON nodes(graph_id);

-- Edges between taint nodes for the graph
CREATE TABLE edges
(
    src     INTEGER NOT NULL,
    dst     INTEGER NOT NULL,
    -- graph.methods.id of the method `dst` sits in.
    --
    -- Nodes are shared between graphs and between analyses, so this cannot live on the node: it
    -- would record whichever analysis happened to create the node first. An edge is always
    -- written by a worker that is inside one specific method, so it always has a true value.
    -- The same flow found in two methods is two rows, which is why location is in the key.
    location INTEGER NOT NULL,

    PRIMARY KEY(src, dst, location),

    FOREIGN KEY(src) REFERENCES nodes(id) ON DELETE CASCADE ON UPDATE CASCADE,
    FOREIGN KEY(dst) REFERENCES nodes(id) ON DELETE CASCADE ON UPDATE CASCADE

) WITHOUT ROWID;


CREATE INDEX edge_dst ON edges(dst);

-- An input parameter by smali register number: p1 is register 1
CREATE TABLE source_params
(
    id       INTEGER NOT NULL,
    register INTEGER NOT NULL,

    PRIMARY KEY (id)
);

-- The result of a call made inside the analyzed method
CREATE TABLE source_calls
(
    id     INTEGER  NOT NULL,
    -- NULL-able because we allow searching without this
    class  TEXT,
    name   TEXT     NOT NULL,
    args   TEXT     NOT NULL,
    -- NULL-able because we allow searching without this
    ret    TEXT,

    PRIMARY KEY (id)
);

-- A field read
CREATE TABLE source_fields
(
    id     INTEGER  NOT NULL,
    class  TEXT     NOT NULL,
    name   TEXT     NOT NULL,

    PRIMARY KEY (id)
);

-- An unknown field write
CREATE TABLE external_field_sinks
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
CREATE TABLE external_call_sinks
(
    id          INTEGER NOT NULL,
    class       TEXT    NOT NULL,
    name        TEXT    NOT NULL,
    signature   TEXT    NOT NULL,
    PRIMARY KEY (id),
    UNIQUE (class, name, signature)
);


CREATE TABLE sink_filters (
    id      INTEGER NOT NULL,
    node    INTEGER NOT NULL,
    kind    TEXT    NOT NULL,
    class   TEXT,
    name    TEXT,
    args    TEXT,
    ret     TEXT,
    PRIMARY KEY (id),
    CHECK (kind IN ('call', 'field', 'ext-field', 'ext-call')),
    UNIQUE(node),
    FOREIGN KEY (node) REFERENCES nodes (id) ON DELETE CASCADE ON UPDATE CASCADE
);

-- Add external and field sinks automatically, but we'll need to add the graph
-- ones in a different way. That is handled by a temporary trigger after attaching.
CREATE TRIGGER update_sink_filters_external
AFTER INSERT ON nodes
WHEN new.kind = 'ext-call'
BEGIN
    INSERT INTO sink_filters (node, kind, class, name, args)
    SELECT
        new.id,
        new.kind,
        external_call_sinks.class,
        external_call_sinks.name,
        external_call_sinks.signature
    FROM external_call_sinks
    WHERE external_call_sinks.id = new.sink_id;
END;

CREATE TRIGGER update_sink_filters_external_field
AFTER INSERT ON nodes
WHEN new.kind = 'ext-field'
BEGIN
    INSERT INTO sink_filters (node, kind, class, name)
    SELECT
        new.id,
        new.kind,
        external_field_sinks.class,
        external_field_sinks.name
    FROM external_field_sinks
    WHERE external_field_sinks.id = new.sink_id;
END;

-- Table that is used by the UI to mark some analyzed_methods as hidden
CREATE TABLE _hidden_analyzed_methods
(
    analyzed_method INTEGER NOT NULL,

    PRIMARY KEY (analyzed_method),
    FOREIGN KEY (analyzed_method) REFERENCES analyzed_methods (id) ON DELETE CASCADE ON UPDATE CASCADE
) WITHOUT ROWID;

-- Table that is used by the UI to mark some graphs as hidden
CREATE TABLE _hidden_graphs
(
    graph   INTEGER NOT NULL,
    PRIMARY KEY (graph),
    FOREIGN KEY (graph) REFERENCES graphs (id) ON DELETE CASCADE ON UPDATE CASCADE
) WITHOUT ROWID;
