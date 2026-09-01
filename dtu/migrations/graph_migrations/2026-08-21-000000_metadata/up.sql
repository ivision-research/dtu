CREATE TABLE _metadata
(
    -- Unix timestamp, UTC, of the last completed build.
    built_at BIGINT NOT NULL,
    -- Unix timestamp, UTC, of the last time anything was deleted from the graph, or null if
    -- nothing ever has been.
    last_delete BIGINT
);

-- Enforces the single row without a column whose only purpose is to say so
CREATE UNIQUE INDEX _metadata_single_row ON _metadata((1));

INSERT INTO _metadata(built_at) VALUES (unixepoch());

CREATE VIEW vsmali_methods AS
SELECT m.id AS id,
       c.id AS class_id,
       s.name AS source,
       c.name || '->' || m.name || '(' || m.args || ')' || m.ret AS smali
FROM methods AS m
JOIN classes AS c
    ON m.class = c.id
JOIN sources AS s
    ON m.source = s.id;

CREATE VIEW vsmali_fields AS
SELECT c.name || '->' || f.name || ':' || f.ty AS smali,
       f.id AS id,
       c.id AS class_id,
       s.name AS source
FROM class_fields AS f
JOIN classes AS c
    ON f.class = c.id
JOIN sources AS s
    ON c.source = s.id;
