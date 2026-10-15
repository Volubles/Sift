CREATE TABLE favorites (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    source_path BLOB NOT NULL UNIQUE CHECK(length(source_path) BETWEEN 1 AND 65536),
    name TEXT NOT NULL CHECK(length(name) BETWEEN 1 AND 512),
    name_key TEXT NOT NULL,
    display_path TEXT NOT NULL CHECK(length(display_path) <= 32768),
    format TEXT NOT NULL CHECK(length(format) BETWEEN 1 AND 16),
    size_bytes INTEGER NOT NULL CHECK(size_bytes >= 0),
    modified_ns INTEGER,
    duration_seconds REAL CHECK(duration_seconds IS NULL OR duration_seconds >= 0),
    sample_rate INTEGER CHECK(sample_rate IS NULL OR sample_rate > 0),
    channels INTEGER CHECK(channels IS NULL OR channels > 0),
    bit_depth INTEGER CHECK(bit_depth IS NULL OR bit_depth > 0),
    metadata_error TEXT CHECK(metadata_error IS NULL OR length(metadata_error) <= 4096)
) STRICT;
CREATE INDEX favorites_name ON favorites(name_key, id);
CREATE VIRTUAL TABLE favorites_fts USING fts5(
    name, display_path, content='favorites', content_rowid='id', tokenize='trigram'
);
CREATE TRIGGER favorites_insert AFTER INSERT ON favorites BEGIN
    INSERT INTO favorites_fts(rowid,name,display_path) VALUES(new.id,new.name,new.display_path);
END;
CREATE TRIGGER favorites_delete AFTER DELETE ON favorites BEGIN
    INSERT INTO favorites_fts(favorites_fts,rowid,name,display_path)
    VALUES('delete',old.id,old.name,old.display_path);
END;
PRAGMA user_version = 2;
