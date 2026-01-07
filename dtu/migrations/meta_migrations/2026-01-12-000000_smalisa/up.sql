CREATE TABLE new_progress (
    step        TEXT    NOT NULL,
    completed   BOOLEAN NOT NULL
);

INSERT INTO new_progress (step, completed) VALUES
    ('pull', (SELECT completed FROM progress WHERE step = 1)),
    ('db-setup', (SELECT completed FROM progress WHERE step = 2)),
    ('graphdb-setup', (SELECT completed FROM progress WHERE step = 3)),
    ('app-setup', (SELECT completed FROM progress WHERE step = 4)),
    ('fast-results', (SELECT completed FROM progress WHERE step = 5)),
    ('emulator-diff', (SELECT completed FROM progress WHERE step = 6)),
    ('acquired-selinux-policy', (SELECT completed FROM progress WHERE step = 7)),
    ('smalisa', 0);

DROP TABLE progress;
ALTER TABLE new_progress RENAME TO progress;
