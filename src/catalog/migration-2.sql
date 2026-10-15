CREATE TABLE folders (
    source_path BLOB PRIMARY KEY,
    parent_path BLOB,
    name TEXT NOT NULL,
    name_key TEXT NOT NULL
) STRICT;
CREATE INDEX folders_parent ON folders(parent_path, name_key, source_path);
PRAGMA user_version = 2;
