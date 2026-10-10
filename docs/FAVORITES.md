# Favorites

Favorites are global saved sound references. They survive detached libraries
and index rebuilds and are independent of collection membership. Removing a
star leaves collection entries and original audio intact.

## Workflow

Stars on Library and Collection rows save/remove favorites. The star icon in
the activity rail opens a dedicated table with its own search. Selection uses
`FavoriteId`, separate from catalog and collection IDs.

Favorites share waveform, playback, source validation, and copy-only dragging
with other views. Switching to Collections allows the selected favorite to be
added there. Watched source changes refresh affected pages; **Refresh favorites**
checks unwatched or offline references.

Stars reflect committed data. Unknown membership disables the control while
loading. Pending edits prevent conflicting changes. Failed saves retain the
mutation and last confirmed state for retry or discard, with feedback available
in each view. Removing an existing favorite requires only its saved path and
works when the source is missing.

## Storage and queries

Favorites share the collection worker and schema-2 durable database. A STRICT
table stores a unique native locator, non-reused ID, saved metadata, and source
revision. Schema-1 migration preserves existing collections and entries;
failure rolls back the upgrade.

Search uses literal trigram FTS5 and indexed short filename prefixes. Results
sort by lowercase filename and ID. Count and rows share a snapshot, with at most
128 returned rows; only those rows inspect source availability. The view retains
four pages and renders visible rows.

Membership reads use indexed lookups for at most 641 paths and 1 MiB of encoded
path data. Cached status covers the loaded Library/Collection pages and selected
source, and changes when those paths or committed edits change. There is no
whole-favorites in-memory set. Separate query/view generations reject stale
responses while retaining scroll identity. The existing four-message worker
queues handle reads and writes without another owner thread.

File location is the current identity boundary. Size and timestamps indicate
revision changes; moves and filesystem aliases are not automatically relinked.
Changed, missing, and inaccessible sources remain visible.

## Qualification

Local verification covers persistence, offline removal, collection independence,
duplicate saves, paging/search, migration rollback, queue recovery, shared star
state, stale replies, and separate star/row hit targets. Native captures cover
all three views and a missing reference with unchanged fixture audio. Physical
star clicks, listening, Explorer drops, and large-favorites latency need broader
qualification. Verification sources are maintained privately.
