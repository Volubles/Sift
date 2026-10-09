# Collections checkpoint

Collections are named saved selections of original sound references. They are
the feature formerly described as projects in Sift's planning. The product UI
uses **Collections**, while **Library** means indexed source sounds.

## Durable storage foundation

`collections::Store` owns a separate SQLite database derived from the catalog
path: `catalog.sqlite` maps to `catalog.collections.sqlite`. Custom catalog paths
produce equally isolated collection stores. Schema 1 has a dedicated application
ID, STRICT tables, foreign keys, WAL, and FULL synchronization. An unknown schema
or a non-collection database is rejected without resetting user data.

Collection and entry IDs are independent typed, non-reused SQLite identities
within this durable database. They do not reference rebuildable catalog row IDs.
Entries preserve the original native path (UTF-16 on Windows, native bytes on
Unix), last-known display/audio metadata, and file revision. Insertion order is
stable. The same source can belong to multiple collections; adding it twice to
the same collection keeps one reference. Removing a reference or collection
never deletes source audio. There is no automatic relinking or transcoding.

Each source is checked on addition. Missing files, directories/reparse points,
invalid fields, and stale indexed file sizes fail visibly. Entry reads distinguish
ready, changed, missing, and inaccessible sources. Revision comparison uses size
and modification time when representable; it is not content identity or proof
against a file changing between checks. Existing missing references are retained.

Collection names are trimmed, limited to 120 characters, and unique by Unicode
lowercase key. Page queries return at most 128 collections or entries. Batch adds
accept at most 128 references and commit transactionally. Path/metadata payloads
have deliberate limits; display text is never a filesystem locator.

The collections worker owns its connection and filesystem checks. Its command
and event queues each hold four messages. Query generations identify stale
responses. GUI mutations can be acknowledged only after commit. On shutdown,
accepted mutation commands drain even if the UI no longer reads their results;
queued reads are skipped after disconnect. The worker blocks when idle.

## Verification and next integration

Tests use generated PCM WAV files and disposable databases. They cover restart,
index rebuild independence, multiple collections, idempotent addition, ordered
IDs, reference removal, unchanged original bytes, changed/missing/inaccessible
sources, transactional rollback, name validation, paging, newer-schema refusal,
and closing with an accepted but unread edit.

Desktop integration follows this foundation: collection creation/rename,
adding/removing references, a bounded left content panel, and source preview.
Internal dragging will add references; native dragging starts only after the
gesture leaves the window. Virtual folders, item reordering, multi-selection,
portable imports/exports, and cross-machine ID merging are later work.
