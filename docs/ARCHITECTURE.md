# Technical direction

This document combines intended architecture and the first implemented slice.
`library`, `catalog`, and `search` provide scanning, metadata probing,
SQLite persistence, and bounded queries. Later sections also describe intended
boundaries for future features. Rust 1.98.1,
rusqlite 0.40.2, and Symphonia 0.6.1 are pinned and verified for this slice.

The desktop slice provides `desktop`, `worker`, and `config`. egui/eframe 0.36.2 renders
the native window; Glow is now the default after the [release baseline](BASELINE.md),
and WGPU remains optional. RFD 0.17.2 supplies the Windows folder dialog. The index
worker owns a writable connection and the search worker owns an independent
read-only connection. Query generations reject stale results. The UI retains
at most four 128-record pages; event/read/index-command queues are bounded at 8/4/4.
The index thread also owns durable named libraries. Folder pages share the read
worker, with their own generations and a bounded desktop tree cache. Selected
source validation has another generation family on that read worker.
Workers block when idle and wake the UI on results. Shutdown cancels scanning,
disconnects channels, and joins workers without an event-queue deadlock.

`desktop.rs` coordinates selection, worker events, and the window's panels.
`desktop::preview` presents playback controls, the waveform, and selected-source
details. `desktop::sound_table` owns the shared Library/Favorites table widgets
and their click/drag interaction tests. Library, collection, and favorite view
state remains in its respective desktop module. Presentation modules perform
no database or file I/O; the existing workers retain those responsibilities.

The [live index](INDEXING.md) adds one native notification consumer, bounded
coalescing, event-loss recovery, and OS thread-pool directory subscriptions.
The consumer blocks when idle; filesystem inspection/SQL remain on the index
owner. Private owned events guarantee shutdown even with no registered roots.

Default runtime storage is in the user's data directory, not the build cache
or launch directory. The desktop/headless feature configurations share the same
core. Playback and waveform analysis are implemented as described in
[preview architecture](PREVIEW.md). The Windows native copy-only adapter and
desktop drag gestures are implemented as described in
[native transfer architecture](DRAG-AND-DROP.md). Saved collection references,
transactional edits, source availability, and the desktop collection panel are
implemented as described in [Collections](COLLECTIONS.md).
The same durable database/worker now owns [Favorites](FAVORITES.md), with indexed
search and bounded membership reads. No additional owner thread is required.

File validation/encoding runs on a bounded preparation worker. The modal OLE
gesture is posted to a private HWND on the window's input thread after the
egui/winit event callback returns; it does not run on a worker input queue or
inside a borrowed application callback. Native references stay thread-local,
and final results wake the UI through a bounded queue.

Catalog schema 3 stores Windows paths as little-endian UTF-16 blobs, preserving
unpaired surrogates; display text is separate. Locators are platform-specific.
Incremental inventories skip unchanged metadata and reconcile missing paths
only after complete enumeration. Sample IDs are non-reused within an index.
Exact native locators are deduplicated across overlapping roots; folder filters
use indexed native ranges. Named library registrations have independent durable
storage, as described in [Libraries](LIBRARIES.md). Native path updates and
completed-inventory deletion reconciliation are described in [Indexing](INDEXING.md).
Durable-source relinking and content/alias identity remain later work.
Unknown metadata is nullable, and failed probes are
stored explicitly. Parser reads are limited to 8 MiB per file.

The initial framework review considered Slint, Iced, GPUI, storage alternatives,
and audio streaming options. egui/eframe is the implemented UI; Slint remains an
alternative if further measurements justify a change. The
[renderer baseline](BASELINE.md) records the local Glow/WGPU comparison.

## Candidates and decision boundaries

| Area | Candidate | Verify before committing to it |
| --- | --- | --- |
| UI | egui/eframe | Resource usage, large lists, waveforms, keyboard interaction, native outgoing drag. |
| Storage | Bundled SQLite with FTS5 through rusqlite | FTS5 availability, transactions, migration, search semantics, locking, actual query plans. |
| Decoding | Symphonia | Real codec variants, seeking, metadata, and long files. |
| Audio device | CPAL | Windows devices, sample-rate adaptation, latency, underruns, and device changes. |
| Resampling | Rubato where needed | Conversion quality, throughput, latency, and rate-change behavior. |
| Audio transport | rtrb SPSC ring buffer | Capacity, underruns, cancellation, buffer generations, and callback safety. |
| Windows integration | Small windows-rs adapter | Copy-only file transfer, COM ownership, multi-selection, and Explorer conflict behavior. |

The backend is embedded in the desktop process. Typed commands and results
connect UI to bounded workers; no local HTTP server or database service is
part of this design. Evaluate WASAPI shared playback first. Rodio and Creek
remain audio baselines/alternatives if they meet the same resource contracts.

A modern tool should provide a concrete improvement. Choose a compatible,
maintained stable toolchain and record its version in `rust-toolchain.toml`
when Cargo is introduced. Start with Rust edition 2024 if the candidates support
it. Track the application's `Cargo.lock`. Nightly, large frameworks, and new
services require actual needs.

Native UI is the starting point. Change the UI choice if prototype evidence
shows another option improves integration or performance. Record a short
decision here or in a focused ADR before a major change. Keep library rules
independent of the UI.

## Responsibilities and dependencies

| Area | Owns | Must not own |
| --- | --- | --- |
| `app` | Startup, configuration, wiring, controlled shutdown | Search rules, decoding, SQL in event handlers. |
| `library` | Roots, file identity, scanning, library state | UI or audio devices. |
| `search` | Queries, filters, stable sorting, result pages | Direct UI widget access. |
| `collections` | Collections, references, ordering, save rules | Modification of library originals. |
| `audio` | Decode, buffering, seek, resampling, playback | Collection persistence and search queries. |
| `analysis` | Waveforms and versioned descriptors | UI thread or unbounded full-file loading. |
| `storage` | SQL, migrations, transaction boundaries | Product decisions and native dragging. |
| `export` | Destination planning, conflicts, copying, outcomes | Source deletion or movement. |
| `platform` | Native dialogs, path semantics, drag-and-drop | Portable domain rules. |
| `ui` | Presentation, interaction, visible state | Blocking I/O and hidden business rules. |

These are responsibility boundaries, not a requirement for ten crates or empty
module files. Begin with one Cargo package and cohesive modules. Extract a
crate only for a real dependency, reuse, or testing need. Portable domain
logic must not depend on egui, CPAL, or Windows objects.

## Data model and file identity

- `LibraryRoot`: stable ID, root locator, availability.
- `Sample`: stable ID, root and relative OS path, file revision, metadata.
- `Collection` / `Entry`: separate IDs, sample references, ordering,
  optional virtual folder, export name, and note.
- Manual tags/favorites: durable user information with an explicit owner.
- Waveforms/analysis: rebuildable derived data with versioned keys.

A content hash is not the sole file identity: different files may contain
identical data and a file may change content. Persistent sample ID, location,
and content revision have different meanings. Document use of OS file IDs,
relative paths, and optional hashes. Ambiguous relinking needs a choice;
missing files retain references and manual data.

Preserve OS paths with `Path`/`PathBuf`. Lossy display conversion must not
feed back into file operations. Choose round-trip-safe path storage for supported
platforms. Do not reduce Windows case/Unicode rules to a globally lowercased
string as file identity.

## Threads and work queues

The UI thread handles presentation and short commands. Disk I/O, SQL, hashing,
decoding, and analysis run in workers with bounded queues, clear ownership,
and controlled shutdown. SQLite connections have deliberate owners; do not
create an unbounded thread or connection per job.

Interaction and audio decoding take priority over analysis. Jobs have
IDs/generations, cancellation, and defined outcomes. Stale search, waveform,
or decode work must not update a newer selection. Coalesce progress events
and apply backpressure instead of flooding the UI.

The audio callback consumes prepared samples from a bounded buffer. It must not
perform file I/O, SQL, logging, heap allocation, or wait on locks/other threads.
On underrun, produce defined silence and update counters read outside the
callback. Seeking/device changes use an explicit state machine and versioned
buffers to prevent old audio entering new playback.

Use synchronous Rust with a small number of workers where sufficient. Introduce
an async runtime only for a concrete integration need. CPU-heavy jobs do not
become inexpensive merely by placing them in an async function.

## Storage and recovery

Separate durable user data from rebuildable library/analysis indexes. Logical
separation may initially share a database if deletion and migrations are clear.
Use versioned migrations, foreign keys, and transactions for multi-step writes.
Test rollback and user-data preservation.

Evaluate WAL and read/write connections against actual workflows. Back up SQLite
using a consistent snapshot method; copying an active database file does not
automatically include uncheckpointed WAL state. An unsupported newer schema
must cause an error, not reset the database.

Distinguish unknown metadata from zero, and errors from pending analysis.
Index actual filter/sort patterns and inspect query plans for large result sets.
Version serialized collections and settings.

Define token, prefix, and substring matching explicitly before choosing FTS5
tokenizers. Verify filename punctuation, Unicode, literal queries, and short
queries; do not substitute unbounded LIKE scans for indexed search by accident.

## Indexes, caches, and similarity

Scan in bounded batches. Hash or decode only when required. Size and modification
time are inexpensive change indicators, not proof of identical content.
File watching needs overflow/reconciliation behavior and an off switch.
An unavailable root is not evidence that the user deleted its contents.

Waveform caches can use multiresolution peaks for large files. Keys include file
revision, algorithm version, and relevant parameters. Every cache needs a size
limit, eviction, and invalidation. No cache may hold the only copy of a collection
or manually entered tags.

If requested, similarity starts with normalized, versioned acoustic descriptors
and measured quality. Compare simple exact search before adding an approximate
vector index; ANN has memory and maintenance costs. Embeddings can be added
later without making AI categorization mandatory.

## Native export

Windows file transfer must offer copying as its only effect and provide proper
native file references. Internal UI dragging is a separate contract. Test
multiple files, Unicode, conflicts, and modifier keys in Explorer.

For application-managed export, validate targets, plan conflicts, and copy in
bounded blocks into a unique temporary file in the destination folder. Publish
the completed file with verified platform semantics; checking that a target
does not exist and then using a normal rename does not itself prevent races or
replacement. Replacement requires an explicit user choice. Targets must not
alias sources. Record per-file success, cancellation, failure, and prior copies.

## Implementation references

Verify documentation against pinned versions when writing code.

- [egui](https://github.com/emilk/egui)
- [SQLite FTS5](https://sqlite.org/fts5.html)
- [Symphonia](https://github.com/pdeljanov/Symphonia)
- [CPAL](https://github.com/RustAudio/cpal)
- [Rubato](https://github.com/HEnquist/rubato)
- [rtrb](https://github.com/mgeier/rtrb)
- [windows-rs](https://github.com/microsoft/windows-rs)
- [Windows Shell transfer](https://learn.microsoft.com/en-us/windows/win32/shell/datascenarios)
