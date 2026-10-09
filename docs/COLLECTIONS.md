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

## Desktop workflow and verification

Tests use generated PCM WAV files and disposable databases. They cover restart,
index rebuild independence, multiple collections, idempotent addition, ordered
IDs, reference removal, unchanged original bytes, changed/missing/inaccessible
sources, transactional rollback, name validation, paging, newer-schema refusal,
and closing with an accepted but unread edit.

The left panel lists collections above their saved sounds. **+** creates a
collection; right-click its name to rename/delete it. **Add selected sound** or
an internal drag onto the collection name adds a reference. Saved sounds can be
selected for waveform/playback and removed via their context menu. Names and
membership changes save after accepted actions. While saving, conflicting edits
are disabled. A failed edit retains its mutation and name draft for retry or
explicit discard; reads fail visibly and retry on request instead of polling.

The library uses a compact, virtualized filename/format/duration/rate/channels
table. Transport/waveform are above it; **Auto-play** previews new selections;
**Details** toggles metadata/source inspection. Library and saved-entry selection
keys are separate types, so a rebuilt catalog cannot accidentally select a saved
entry with the same numeric ID. Missing saved sources remain visible and disable
preview; changed sources show that metadata is from the saved snapshot.

An internal drag remains an egui payload while inside the window. Collection
names receive it as a reference; empty/unsuitable internal areas cancel quietly.
After a held primary drag crosses the client boundary (including PointerGone),
the payload is handed to the existing copy-only Windows adapter. Returning into
Sift after that native handoff cancels; it does not turn into another internal
add. Source files are unchanged. Actual Explorer gestures after this routing
change still need their own physical verification.

The GUI controller tests commit/reload create, add, rename, remove, failed name
draft recovery, and the Enter key that opens a form without submitting it. A
deterministic egui interaction test delivers an internal payload to a collection
target before frame cleanup and distinguishes inside/outside handoff.
Native captures with four generated audio fixtures and pre-seeded disposable
collections were visually checked. Windows accessibility/keyboard operation
created a collection in the actual app. Mouse capture/input geometry was
unavailable in the automation helper, so physical mouse/context-menu/Explorer
verification is not claimed from those tests.

Virtual folders, item reordering, multi-selection, portable imports/exports,
cross-machine ID merging, and restoring the last active collection are later work.
The first saved collection opens initially; all named collections and entries
survive restart regardless of the active view.
