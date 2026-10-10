# Incremental indexing and live changes

## Inventories

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

## Live Windows updates

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

## Qualification and limits

Local verification covers unchanged inventories, timestamp-preserving edits,
renames/moves/deletions, saved references, incomplete scans, offline roots,
native notifications, pause/resume, source states, overflow recovery, and shutdown.
The fixtures and verification sources are maintained privately.

The [100,000-file measurement](INDEXING-BASELINE.md) records generated WAV
inventory timings and native update latency. Worker readiness, indexed results,
rendered interaction, and audible output are distinct milestones.

Full inventories enumerate directories and compare inexpensive signatures.
Size/timestamps are not content hashes and cannot detect every offline edit
that deliberately preserves them. Notifications can be coalesced or lost;
recovery converges after changes settle rather than providing an atomic
filesystem snapshot. External drives, NAS, volume replacement, and broad
notification-failure conditions still need qualification.

The pinned `windows-file-watcher` backend reports loss explicitly and preserves
native UTF-16 names. Its notification queue and owned Windows events support
blocking consumption and controlled shutdown. See
[backend documentation](https://docs.rs/windows-file-watcher/0.2.2/windows_file_watcher/).
