# Named libraries and folder browsing

A library is a named bank of existing source folders. Collections are saved
sound references. Registering, renaming, or removing either never moves or
deletes source audio. A bank may span disks and contain overlapping folders;
the catalogue stores each exact native file locator once.

## Storage foundation

Library names and source registrations live in a separate schema-1 database,
`catalog.libraries.sqlite` beside the selected catalogue. Typed, non-reused
library/source IDs, lossless native paths, foreign keys, transactions, and full
SQLite synchronization protect accepted changes. An index rebuild can leave
this file and the collection database intact. The prototype bounds registrations
to 128 banks, 32 sources per bank, and 512 sources overall.

Catalog schema 2 adds a global native-locator index and a derived folder table.
The transactional migration keeps the oldest ID for duplicate locations,
updates FTS through its existing triggers, and builds the folder tree in
128-record batches. Invalid locators abort migration without advancing its
version. Unknown schema versions are rejected without resetting data.

Folder queries use separator-terminated native-byte ranges on the source index,
including descendants without matching similarly named sibling folders. Nested
and repeated roots are coalesced. Search and count share a transaction snapshot;
folders have bounded 64-row child pages. Display strings never locate files.
Folders with indexed audio form the tree; empty directories are omitted.

Previous catalogue roots can be imported once without opening or rescanning
their files. An import marker prevents removed banks from reappearing on restart.
Offline sources retain cached records and registrations. Availability checks run
on a worker when snapshots refresh, not on every frame.

## Browser checkpoint

The desktop integration provides Libraries and Collections in a narrow
activity rail. The library sidebar shows named banks above their source/folder
tree. Selecting a bank shows all its indexed sounds; selecting a folder scopes
the central table and search to that subtree. `All files` restores the bank
scope. Switching to Collections preserves the library, folder, and query.

Create/rename/remove bank and attach/detach source operations run on the storage
worker. Attaching registers the directory before scanning; cancelling the scan
retains the registration and committed batches. Removing a registration leaves
its index rows as rebuildable cache, excluded from registered-bank queries.

The existing index thread owns both the catalog writer and the durable library
connection. The independent read thread serves search and folder pages. Their
command/read/event queues are bounded at 4/4/8. Query and tree generations reject
late answers; accepted edits drain on shutdown even when completion is unread.
The UI holds one 64-row page per open folder, with at most 32 open branches and
visible-row rendering. Collapsing a branch releases its descendant pages.
Failed edits retain their command and name draft for correction, retry, or discard.
Changing sidebar mode leaves the library scope and search text intact. No new
framework, service, runtime, or application thread was introduced.

The CLI `index` command also registers its source as a named bank, reusing an
existing registration. `browse --folder` attaches to the selected bank or
creates `My library` for an empty registry. These developer entry points use
the same source validation and durable storage.

## Limits and evidence

The storage tests cover overlapping roots, stable sample IDs, native Unicode
component boundaries, folder paging, schema-1 migration/rollback, restart,
index rebuild, offline sources, source preservation, and one-time legacy import.
Desktop tests cover failed-draft recovery, form-opening Enter behavior, stale
tree answers, scoped worker responses, and unread-save shutdown. All five
generated WAV/FLAC/MP3/AIFF fixture hashes remain unchanged after indexing.
A native release capture rendered the two-bank browser and four-row selected
bank; it was visually inspected before the final painted-arrow/row polish.
The Windows helper could read controls but could not click an indexed element;
Computer Use was then stopped with physical Escape. Folder/bank mouse selection,
source dialogs, Collections mode switching, and Explorer gestures remain manual
runtime checks. The final build is compiled; no new CPU/RAM qualification is
claimed.

The [incremental live index](INDEXING.md) now skips unchanged metadata and
reconciles moved/deleted cached paths. Durable source relinking, favourites, multi-library
selection, and restart persistence of the active view are later checkpoints.
Exact locators are deduplicated; distinct hard links or filesystem aliases are
not a content-hash identity system. No performance improvement is claimed from
the data structure alone.
