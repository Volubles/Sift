-- Keep sample IDs, but never reuse an ID after live reconciliation deletes it.
DROP TRIGGER samples_insert;
DROP TRIGGER samples_delete;
DROP TRIGGER samples_update;
ALTER TABLE samples RENAME TO samples_v2;
CREATE TABLE samples (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    root_id INTEGER NOT NULL REFERENCES library_roots(id),
    source_path BLOB NOT NULL,
    name TEXT NOT NULL,
    name_key TEXT NOT NULL,
    path TEXT NOT NULL,
    format TEXT NOT NULL,
    size_bytes INTEGER NOT NULL CHECK(size_bytes >= 0),
    duration_seconds REAL,
    sample_rate INTEGER,
    channels INTEGER,
    bit_depth INTEGER,
    metadata_error TEXT,
    modified_ns INTEGER,
    created_ns INTEGER,
    probe_version INTEGER NOT NULL DEFAULT 0
) STRICT;
INSERT INTO samples(id,root_id,source_path,name,name_key,path,format,size_bytes,
    duration_seconds,sample_rate,channels,bit_depth,metadata_error)
SELECT id,root_id,source_path,name,name_key,path,format,size_bytes,
    duration_seconds,sample_rate,channels,bit_depth,metadata_error FROM samples_v2;
DROP TABLE samples_v2;
CREATE INDEX samples_name ON samples(name_key,id);
CREATE UNIQUE INDEX samples_source ON samples(source_path);
CREATE TRIGGER samples_insert AFTER INSERT ON samples BEGIN
    INSERT INTO samples_fts(rowid,name,path) VALUES(new.id,new.name,new.path);
END;
CREATE TRIGGER samples_delete AFTER DELETE ON samples BEGIN
    INSERT INTO samples_fts(samples_fts,rowid,name,path) VALUES('delete',old.id,old.name,old.path);
END;
CREATE TRIGGER samples_update AFTER UPDATE ON samples
WHEN old.name IS NOT new.name OR old.path IS NOT new.path BEGIN
    INSERT INTO samples_fts(samples_fts,rowid,name,path) VALUES('delete',old.id,old.name,old.path);
    INSERT INTO samples_fts(rowid,name,path) VALUES(new.id,new.name,new.path);
END;
ALTER TABLE folders ADD COLUMN prefix_lower BLOB;
ALTER TABLE folders ADD COLUMN prefix_upper BLOB;
PRAGMA user_version=3;
