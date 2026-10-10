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

## Live Windows changes

The index owner registers coalesced source roots with `windows-file-watcher`
0.2.2 (MIT, Rust 1.98). It uses native directory notifications, not recurring
whole-library polling. One notification thread blocks on the native queue event
and a private shutdown event. A small, documented Windows FFI boundary owns
those event handles; no UI/audio object enters the notification thread.

The native session allows 512 standing source reservations plus 64 notification
batches. Sift's pending set holds at most 1,024 paths and 1 MiB of path bytes.
Repeated paths coalesce; notifications settle for 200 ms, with a 1 s maximum
burst window. Pending-set overflow and backend desynchronization promote the
affected root to a bounded recovery inventory. Event-loss recovery re-probes
metadata once, since lost events can hide timestamp-preserving writes. The
ordinary path checks only the affected file/subtree. Access events do not cause
continuous rescans.

Source subscriptions are installed before startup reconciliation. Offline edits
are discovered by the startup inventory; paused updates catch up when resumed.
Unavailable subscriptions retry opening with 2–30 s backoff without enumerating
the library. Changed volumes stop the affected subscription and retain cached
records pending explicit reconnection. Root relocation/relinking remains a
separate decision; directory/file changes within registered roots are reconciled.

Accepted library edits drain during shutdown, even with unread results. Ordinary
commands take priority before another background batch. Cancellation pauses live
updates; the checkbox resumes them. Queues remain bounded at 4/4/8 for index
commands, reads, and desktop events. Native callbacks never wait for that queue.

The desktop preserves the library scope, query, selection, and scroll identity
when data refreshes. New rows become visible during long scans, with query
refresh throttled to 250 ms while published changes continue. Selected sources
are validated on the read worker using separate generations. Actual changes
refresh metadata/waveforms and stop a stale preview; missing sources retain their
references and become unavailable. Observed writes can bypass waveform cache
signatures, including size/timestamp-preserving content changes.

## Verification and limits

Generated WAV tests cover unchanged/restarted inventories with no main-database
write, timestamp-preserving edits, additions, nested moves/deletions, non-reused
IDs, saved-reference preservation, cancelled/incomplete scans, offline roots,
and rejecting paths outside approved roots. Earlier schema migration, rollback,
FTS, collection, and source-preservation tests remain applicable.

Actual Windows tests cover creation, timestamp-preserving overwrite, ordinary
and case-only rename, moves to new subdirectories, deletion, partial media that
becomes valid, pause/resume catch-up, source status, and bounded shutdown with
no roots. Controlled coalescing tests cover duplicate hints and bounded overflow;
metadata/waveform tests cover explicit event-loss recovery and preserved stamps.
A release Windows capture was visually checked. These checks do not certify
every external disk, NAS, volume replacement, or file-watcher failure mode.

See [the 100,000-file measurement](INDEXING-BASELINE.md) for actual generated WAV
inventory timings, native update latency, and the limits of the idle observation.
Warm startup of the index worker is distinct from a usable rendered desktop.
Notification-to-index timing is distinct from interaction or audible latency.

Full inventories necessarily enumerate directories and compare cheap signatures.
Signatures are not content hashes and cannot prove content identity after an
offline edit that deliberately preserved size/timestamps. AI analysis, exact
hash deduplication, durable reference relinking, and broad disk/device support
are separate checkpoints. Watch notifications can be coalesced or lost; they
are not an atomic filesystem snapshot. Recovery converges after changes settle.

## Dependency decision

The pinned backend was checked against its shipped source, feature flags,
license, shutdown/retry protocol, and live Windows behavior. Its queue-mediated
native design reports OS/client losses explicitly and preserves native UTF-16
names. This fit the Windows-first prototype without an async runtime or service.
The initially evaluated notify 8.2.0 backend ignores `_bytes_written` in its
Windows completion handler and does not report every loss path required here;
it was removed before delivery. Relevant primary sources:
[backend documentation](https://docs.rs/windows-file-watcher/0.2.2/windows_file_watcher/),
[notify 8.2.0 Windows source](https://docs.rs/crate/notify/8.2.0/source/src/windows.rs).
