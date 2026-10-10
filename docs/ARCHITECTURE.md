# Architecture

Sift is one Rust application with an embedded catalog and a native desktop UI.
The desktop and command-line tools share the same indexing and storage code.
All core workflows run locally without accounts or network services.

## Components

| Module | Responsibility |
| --- | --- |
| `desktop` | Window layout, navigation, catalog events, and selection between views |
| `desktop::preview` | Selected-source state, source validation generations, waveform results, playback controls, and details |
| `desktop::selection` | View-local source marks, range generations, keyboard focus, and pending navigation |
| `desktop::library_view` | Current library/folder query, selection, and remembered sound-table views |
| `desktop::navigation` | Bounded inactive view caches shared by the library, tree, and collection owners |
| `desktop::libraries` | Library registrations, folder scopes, breadcrumbs, and tree navigation |
| `desktop::collections` / `favorites` | Saved-reference views and pending edits |
| `library` | Source enumeration, metadata probing, revisions, and synchronization |
| `catalog` / `search` | Searchable metadata, folder queries, and query semantics |
| `libraries` / `collections` | Durable library registrations and saved references |
| `worker` | Catalog read/index owners and bounded desktop requests |
| `audio` | Decoding, rate conversion, buffering, and output-device ownership |
| `waveform` | Peak analysis and derived cache |
| `watch` | Native Windows notifications, coalescing, and recovery |
| `platform` | Copy-only Windows file transfer and its native message host |

`Preview` owns its waveform and playback workers. Selection, source revisions,
errors, and request generations live with the transitions that update them.
The window reads the selected reference through that owner's interface; late
source, waveform, and playback replies are rejected within the preview flow.

## Storage

The Windows default directory is `%LOCALAPPDATA%\Sift\`. A custom catalog path
produces corresponding companion databases.

| Database | Data | Lifetime |
| --- | --- | --- |
| `catalog.sqlite` | Schema-3 metadata, native locators, FTS5 index, and derived folders | Rebuildable |
| `catalog.libraries.sqlite` | Schema-1 named libraries and source registrations | Durable |
| `catalog.collections.sqlite` | Schema-2 collections, entries, and global favorites | Durable |
| `catalog.waveforms.sqlite` | Versioned min/max peaks keyed by source revision | Rebuildable |

Durable IDs are independent of catalog row IDs. Rebuilding the catalog leaves
libraries, collections, and favorites intact. Schema changes use transactions;
unsupported versions and failed migrations return errors without resetting data.

SQLite connections have explicit worker owners. The index owner has a writable
catalog connection and owns library registration changes. Search uses an
independent read-only connection. A separate collection worker owns saved edits
and favorite reads. Accepted durable changes drain during shutdown even when
completion events are no longer read by the UI.

Collection group commands carry the captured query, folder scope, native marks,
and destination ID. The collection actor resolves the full marked set before
saving, using one lazy read-only catalog connection for library selections.
Favorites and collection inputs use its durable connection. Resolution returns
at most 128 references; larger or changed explicit sets fail without saving a
prefix. Removal resolves membership and deletes in the same transaction without
inspecting source files. Accepted group commands also drain during shutdown.

## Paths and identity

Windows locators are stored as little-endian UTF-16 bytes, including unpaired
surrogates. Unix locators use native bytes. Display text is separate from paths
used for file operations. Exact native locators are deduplicated across
overlapping roots; aliases and hard links remain distinct locations.

File location, catalog ID, and content identity are separate concepts. Current
revision checks use file size and available timestamps. Missing sources retain
saved references. Automatic relinking and content-hash deduplication are not
implemented.

## Background work

Catalog event/read/index-command queues hold 8/4/4 messages. Search results have
128 rows per page; the library view retains four pages. Folder queries return
64 children, and the desktop keeps at most 32 branches open. Query, folder, and
selected-source generations prevent old replies from replacing newer state.

Selection is separate from preview. Each sound view owns its marks and focus.
Ctrl+A is a compact view-wide intent; explicit ranges use native path endpoints
and resolve on existing workers in one SQL snapshot. Replies and mark sets have
path-count/byte bounds. Generations reject obsolete ranges, while marking and
keyboard navigation leave durable data and original audio untouched. See
[Selection](SELECTION.md).

Internal drags carry an immutable selection intent. The add button uses the last
used sound list; selecting a collection destination retains one bounded prior
collection snapshot until the user makes new marks there. SQL and source checks
remain off the UI thread, and failed group edits retain the captured command for
retry or discard.

Session view state stays with each view owner. Library/folder queries and marks
move into `LibraryView`'s cache; library trees and collection contents have their
own caches. Each retains at most 16 inactive views and 8 MiB of estimated state.
Pages, audio data, workers, and pending edits are excluded. Generations remain
monotonic across restoration; unfinished navigation/ranges are cancelled. The
preview owner prepares restored references silently and verifies their native
sources. See [Navigation](NAVIGATION.md) for user-visible behavior and limits.

Workers wait for requests or native signals while idle and request a repaint
when results arrive. The UI performs presentation and submits short commands;
file inspection, SQL, decoding, and peak analysis run outside it. Playback uses
timed repaints only while loading or playing.

The Windows watcher has one notification consumer and native directory
subscriptions. Coalescing, explicit notification-loss recovery, pause/resume,
and offline-root handling are described in [Indexing](INDEXING.md).

## Audio and native integration

The playback worker owns decoding and the CPAL device. Its callback consumes
preallocated ring-buffer frames and atomics. Device callbacks perform no file
I/O, logging, allocation, or blocking synchronization. Buffer sizes, conversion
rules, and output limits are described in [Preview](PREVIEW.md).

Outgoing dragging validates references on a preparation worker, then posts the
OLE gesture to a private HWND on the window's input thread. Native objects stay
on that thread, and callback pointers retain their owning state through nested
message delivery. Native drag and watcher FFI are the scoped unsafe boundaries;
safety comments document their handle, pointer, and thread contracts.
See [Drag and drop](DRAG-AND-DROP.md).

The same preparation worker resolves captured library/favorite/collection marks
through lazy read-only catalog and saved-reference connections. It preserves
query/scope identity across UI changes and validates the complete group before
encoding. Its cancellation token spans resolution, file inspection, encoding,
and native dispatch; released or cancelled gestures never restart from late
ready results. The UI consumes each internal drag payload once at handoff.

## Build configuration

Catalog schema 4 and saved-reference schema 3 add indexed sound ordering.
Page reads, ranges, and native cursor/anchor lookup share a typed ordering rule;
numeric unknowns sort last. Existing saved entry IDs and insertion order survive
the transactional migration. See [Sound ordering](SORTING.md).

`rust-toolchain.toml` pins Rust 1.98.1; the package uses edition 2024 and tracks
`Cargo.lock`. The desktop uses egui/eframe 0.36.2 with Glow, and RFD 0.17.2 for
the Windows folder picker. SQLite is bundled through rusqlite 0.40.2.
Symphonia 0.6.1, CPAL 0.18.2, Rubato 5.0.1, and rtrb 0.3.3 supply audio support.
The native adapters use windows-rs 0.62.2 and windows-file-watcher 0.2.2.

Default features enable the desktop, playback, and native dragging.
`--no-default-features` provides the CLI-only configuration. WGPU is optional
through `wgpu-renderer` and selected with `SIFT_RENDERER=wgpu`; Glow remains the
default. The [renderer comparison](BASELINE.md) records the measurement basis.

Verification sources are maintained privately. The build script enables unit
hooks when `tests/unit/` exists; a source-only checkout builds without those
files. This compile-time configuration adds no application runtime dependency.
