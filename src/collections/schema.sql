CREATE TABLE collections (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL CHECK(length(name) BETWEEN 1 AND 120),
    name_key TEXT NOT NULL UNIQUE
) STRICT;

CREATE TABLE entries (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    collection_id INTEGER NOT NULL REFERENCES collections(id) ON DELETE CASCADE,
    source_path BLOB NOT NULL CHECK(length(source_path) BETWEEN 1 AND 65536),
    name TEXT NOT NULL CHECK(length(name) BETWEEN 1 AND 512),
    display_path TEXT NOT NULL CHECK(length(display_path) <= 32768),
    format TEXT NOT NULL CHECK(length(format) BETWEEN 1 AND 16),
    size_bytes INTEGER NOT NULL CHECK(size_bytes >= 0),
    modified_ns INTEGER,
    duration_seconds REAL CHECK(duration_seconds IS NULL OR duration_seconds >= 0),
    sample_rate INTEGER CHECK(sample_rate IS NULL OR sample_rate > 0),
    channels INTEGER CHECK(channels IS NULL OR channels > 0),
    bit_depth INTEGER CHECK(bit_depth IS NULL OR bit_depth > 0),
    metadata_error TEXT CHECK(metadata_error IS NULL OR length(metadata_error) <= 4096),
    UNIQUE(collection_id, source_path)
) STRICT;
CREATE INDEX entries_collection_order ON entries(collection_id, id);
PRAGMA application_id = 1397113676;
PRAGMA user_version = 1;
