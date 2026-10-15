CREATE TABLE tags (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL CHECK(length(name) BETWEEN 1 AND 40),
    name_key TEXT NOT NULL UNIQUE
) STRICT;
CREATE TABLE sound_tags (
    source_path BLOB NOT NULL CHECK(length(source_path) BETWEEN 1 AND 65536),
    tag_id INTEGER NOT NULL REFERENCES tags(id) ON DELETE CASCADE,
    PRIMARY KEY(source_path, tag_id)
) STRICT, WITHOUT ROWID;
CREATE INDEX sound_tags_by_tag ON sound_tags(tag_id, source_path);
PRAGMA application_id = 1397118023;
PRAGMA user_version = 1;
