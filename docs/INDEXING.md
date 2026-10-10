# Incremental indexing and live changes

## Incremental foundation

Catalog schema 3 preserves previous sample IDs and makes future IDs non-reused.
Metadata records include size, modified/creation timestamps, and a probe version.
An unchanged inventory skips audio probing and persistent sample/FTS writes.
Unknown timestamps are never assumed unchanged. Native write hints force probing,
including edits made by tools that preserve timestamps. Changed files are checked
again after probing; unstable reads are deferred instead of publishing stale data.

An inventory records seen native paths in a disk-backed temporary SQLite table,
with a 1 MiB temp page cache and 64-file batches. Only a completed enumeration
removes unseen cached rows and empty derived folders. Cancellation, inaccessible
branches, depth limits, unavailable/replaced roots, and SQL failures preserve
previous rows. Corrupt audio remains an explicit metadata-error record. Neither
source audio nor independently saved collections are deleted or rewritten.

Single-path synchronization inspects current filesystem state, validates the
registered root and ancestors, and only updates that file or subtree. A missing
subtree can remove its cached metadata while an offline root retains its cache.
Renames appear at their new location; saved collection references remain intact
and may become missing. Automatic durable-source relinking requires a separate
identity decision and is not guessed from filenames.

## Verification and next checkpoint

Generated WAV tests cover unchanged/restarted inventories with no main-database
write, timestamp-preserving edits, additions, nested moves/deletions, non-reused
IDs, saved-reference preservation, cancelled/incomplete scans, offline roots,
and rejecting paths outside approved roots. Earlier schema migration, rollback,
FTS, collection, and source-preservation tests remain applicable.

Live Windows watching, bounded burst coalescing, recovery from lost events,
desktop refresh, and the 100,000-file measurements are the next checkpoint.
