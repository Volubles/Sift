CREATE TABLE library_roots (
    id INTEGER PRIMARY KEY,
    source_path BLOB NOT NULL UNIQUE
) STRICT;

CREATE TABLE samples (
    id INTEGER PRIMARY KEY,
    root_id INTEGER NOT NULL REFERENCES library_roots(id),
    source_path BLOB NOT NULL,
    name TEXT NOT NULL,
    name_key TEXT NOT NULL,
    path TEXT NOT NULL,
    format TEXT NOT NULL,
    size_bytes INTEGER NOT NULL CHECK (size_bytes >= 0),
    duration_seconds REAL,
    sample_rate INTEGER,
    channels INTEGER,
    bit_depth INTEGER,
    metadata_error TEXT,
    UNIQUE (root_id, source_path)
) STRICT;

CREATE INDEX samples_name ON samples(name_key, id);
CREATE VIRTUAL TABLE samples_fts USING fts5(
    name, path, content='samples', content_rowid='id', tokenize='trigram'
);

CREATE TRIGGER samples_insert AFTER INSERT ON samples BEGIN
    INSERT INTO samples_fts(rowid, name, path) VALUES (new.id, new.name, new.path);
END;
CREATE TRIGGER samples_delete AFTER DELETE ON samples BEGIN
    INSERT INTO samples_fts(samples_fts, rowid, name, path)
        VALUES ('delete', old.id, old.name, old.path);
END;
CREATE TRIGGER samples_update AFTER UPDATE ON samples BEGIN
    INSERT INTO samples_fts(samples_fts, rowid, name, path)
        VALUES ('delete', old.id, old.name, old.path);
    INSERT INTO samples_fts(rowid, name, path) VALUES (new.id, new.name, new.path);
END;

PRAGMA user_version = 1;
