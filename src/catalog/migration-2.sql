-- A physical locator is indexed once, even when registered roots overlap.
-- Keep the oldest sample ID; saved collections use independent native locators.
DELETE FROM samples WHERE id NOT IN (SELECT min(id) FROM samples GROUP BY source_path);
CREATE UNIQUE INDEX samples_source ON samples(source_path);

CREATE TABLE folders (
    source_path BLOB PRIMARY KEY,
    parent_path BLOB,
    name TEXT NOT NULL,
    name_key TEXT NOT NULL
) STRICT;
CREATE INDEX folders_parent ON folders(parent_path, name_key, source_path);
PRAGMA user_version = 2;
