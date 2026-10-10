# Current status and next steps

Last updated: 2026-10-10. This document describes actual deliveries; product
and architecture documents describe the intended application.

## Completed

- Local Git repository on `main`, repository hygiene, Conventional Commit workflow.
- AGENTS.md with durable quality requirements and instructions for resuming work.
- Requirements for libraries, search, preview, collections, and copying.
- Technical direction, responsibilities, threads, storage, native boundaries.
- Proposed performance budgets and a reproducible measurement plan.
- Repository documentation translated to English; English is required for
  product UI and all user-visible content. Conversation remains Swedish.
- [Stack review](STACK-REVIEW.md) covering current UI, storage, audio, and native
  integration alternatives with primary-source evidence and prototype gates.

- First implementation: Cargo package, pinned Rust 1.98.1, metadata scanner,
  SQLite/FTS5 catalog, bounded search, lossless native paths, and a runnable CLI.
- Eleven automated checks cover real WAV metadata, unchanged sources, cancellation,
  rollback, schema protection, search semantics, paging, and native path encoding.
- Native egui/WGPU desktop browser with folder selection, search, metadata
  inspection, cancellation, independent workers, and a bounded page cache.
- Additional tests cover concurrent visible pages, worker startup failure,
  full-queue shutdown, and retaining pages when scrolling backward.

Playback and waveforms are now implemented with a bounded audio worker and
derived waveform cache. Single-source copy-only native dragging is integrated
on Windows. Saved collections and global favorites are implemented. Tags,
multi-selection, and application-managed export are not implemented yet. No CI
configuration exists yet. Active implementation branch: `prot/library-prototype`;
check Git before assuming a branch in a future session.
No remote, publishing, or distribution license has been selected.

## Decisions and open choices

- Agreed: Rust, efficient local workflow, collections as references, copy-only
  outgoing drag-and-drop, English artifacts, and small checkpoint commits.
- Reviewed prototype direction: Windows first, egui/eframe, bundled SQLite/FTS5
  through rusqlite, Symphonia, CPAL, Rubato when needed, rtrb, and a limited
  windows-rs adapter. Slint is the main UI alternative. Validate candidates
  before selecting dependency versions or claiming comparative performance.
- Open: visual design, exact format matrix, device/backend support, approved
  sample collection, final performance budgets, and distribution/license.
- Similarity search is later and optional; AI categorization is outside the core.

## Verification

Favorites are implemented in durable collection schema 2 with independent
IDs/native references, transactional upgrade, indexed search, bounded membership
reads, and shutdown-safe edits. Library/Collection rows share star status; the
third rail icon opens the global virtualized Favorites view with independent
search and existing preview/copy routes. Pending/failed edits remain visible
for retry/discard; missing favorites remain saved and can be removed offline.
The integrated suite passes 86 default-feature tests, 59 headless tests,
formatting, Clippy, and a Windows release build. Real release captures of
Libraries/Collections/Favorites were visually checked and five generated source hashes are
unchanged. Physical star clicks, listening, and Favorite-to-Explorer drops remain
manual checks; no new full performance qualification is claimed.
See [Favorites](FAVORITES.md).

The repository foundation was checked for UTF-8/line endings, local links,
ignore behavior, and text/binary attributes. Review staged diffs at each
checkpoint. Git history is the source of current hashes and branch.

The stack review checked official docs, release-tag source, and current crate
metadata. It did not install tools or compile candidate frameworks.

The initial metadata-browser checkpoint compiled and passed fifteen default-
feature tests, fourteen headless tests, Clippy, and formatting. An actual Windows window has
rendered real generated fixtures and a 100,000-row synthetic catalog with only
128 records loaded into the view. Captures were visually checked.
The full interactive Explorer flow and complete performance/device matrix remain
unverified. See [preview verification](PREVIEW.md) and
[native drag verification](DRAG-AND-DROP.md) for subsequent evidence and limits.
The CLI also indexed generated WAV, FLAC, MP3, and AIFF fixtures without errors;
all four SHA-256 source hashes were unchanged afterward.

## Next proposed task

Incremental indexing and live Windows updates are implemented in catalog schema 3. Unchanged inventories
skip audio probes and persistent sample/FTS writes, while completed inventories
remove missing cache rows and empty derived folders. Cancelled/incomplete scans
and offline roots retain previous data. Native path updates reconcile a changed
file or subtree and force probing for timestamp-preserving writes. Non-reused
sample IDs prevent a deleted selection from attaching to a new file; saved
collections remain independent. Generated fixture tests cover these rules.
Native directory watching detects additions, writes, moves/renames, and removals,
with bounded settling/coalescing and explicit loss recovery. Offline sources
retain cached data; paused watching catches up on resume. The desktop preserves
scope/query/selection/scroll through updates and validates selected sources on
the read worker before refreshing metadata/waveforms. Source writes can bypass
waveform cache signatures. A native release capture was visually checked.
See [Indexing](INDEXING.md) for threads, limits, dependency choice, and semantics.

74 desktop tests pass, including actual Windows filesystem events; 52 headless
tests, formatting, and Clippy pass. The real generated 100,000-WAV release
harness also passed. Individual additions inspected one audio file, with
median/p95/max 235/238/241 ms; a ten-file burst took 279 ms. Unchanged inventory
took 2.4–2.8 s against 15–16 s full-refresh repeats in the prior release.
Initial import is still slower (21.8 s after batching directory inserts; prior
initial 14.8 s) and remains an optimization target. The index/watcher component
averaged 19.56 MiB private bytes and had no measurable CPU-time increase over
56.7 sampled seconds inside a 60-second no-scan observation. This excludes
desktop/GPU/audio and is not full idle-budget qualification. See
[measurement](INDEXING-BASELINE.md).

Named libraries and the folder browser are implemented. The drive/grid activity
rail switches Libraries and Collections while preserving the current bank,
folder, and query. Banks support create/rename/remove and multiple source folders;
folder clicks include descendants, and `All files` restores the bank scope.
Source registration is saved before scanning; scan cancellation keeps it.
Bank/source edits preserve original audio and independently saved collections.
The existing index thread owns durable libraries; the read thread serves scoped
search and folder pages with separate generations. The tree caches at most 32
64-row pages and renders visible rows. No application thread or dependency was
added. See [Libraries](LIBRARIES.md).

Catalog schema 2 deduplicates exact native locators across overlapping roots and
migrates previous metadata transactionally. The migration builds every original
root hierarchy before removing duplicate rows. Durable registrations survive
index rebuilds; offline sources stay visible. Native component scopes, paging,
migration rollback, one-time legacy import, restart, failed-draft correction,
form Enter behavior, stale responses, and unread-save shutdown have coverage.
The default suite passes 60 tests; 38 headless tests, formatting, and Clippy
also pass. Five generated WAV/FLAC/MP3/AIFF source hashes are unchanged after
real indexing. A release Windows capture rendered two banks, the source tree,
and the four-row selected bank; it was visually inspected before final painted
arrow/row polish. The release executable is rebuilt. The Windows helper read
controls but could not click an indexed element; Computer Use was stopped with
physical Escape. Source dialogs, bank/folder mouse selection, mode switching,
and Explorer gestures remain manual checks. CPU/RAM have not been qualified
again for this checkpoint.

The first release baseline found excessive private memory with WGPU; a
controlled Glow comparison reduced it from about 477 to 134 MiB. Glow is now
the default, with WGPU retained as an optional renderer. See [baseline](BASELINE.md).

Preview integration is complete: 26 default-feature and 23 headless tests pass,
plus one explicitly run real-device concurrency test. Four generated formats
passed actual WASAPI callback exercises with zero final-run underruns and
unchanged source hashes. Interactive listening and device unplug/replug are
still manual checks; internal callback timing is not audible-latency proof.

Post-preview no-input observation: approximately 77 MiB private bytes over
30 seconds with no measurable process CPU-time increase. Two higher-CPU runs
were not controlled for interaction; diagnostics detected active input in one.
The full idle budget and device/codec matrix are not certified from these runs.

The Windows copy-only adapter and desktop gestures are implemented. Drag a row
or **Drag copy to folder** into a destination; Escape cancels. The actual Shell
target test copied three disposable WAV files, including Unicode and a long
path, with unchanged originals. Encoding, bounds, cancellation, ownership, and
click/drag/reset tests pass. The complete integrated build passes 34 default-
feature and 23 headless tests, plus separate Shell integration and seven adapter-
only checks; formatting and Clippy pass. The release native window capture was
visually checked. See [native drag checkpoint](DRAG-AND-DROP.md).

The first manual gesture stayed in the busy message without native cursor
feedback. Escape did not finish it while other controls remained usable. That
attempt exposed the original worker's unrelated input queue; isolated Shell
copying and a simulated egui gesture had not proved the complete native flow.

The repair keeps filesystem preparation on its bounded worker and posts OLE
startup to a private message-only HWND on the Sift window's input thread, after
the current egui/winit callback returns. Native ownership is thread-local; job
cancellation, late release, final result delivery, and busy-state reset are
explicit. Regression tests include actual owner-queue delivery with cancellation
before the queued native start. The repaired build passes 37 default-feature
tests, ten adapter-only tests, Shell copying, formatting, and Clippy. The default
release binary is rebuilt and its native window smoke check passes.

The Windows automation helper failed to initialize (OS error 3), so repaired
Sift-to-Explorer physical mouse gestures, repeated drops, modifiers, conflicts,
and playback during dragging still need runtime verification. Do not claim
that full flow from the queue/Shell tests alone.

Drag feedback now ends quietly after acceptance, Escape, or an unsuitable target;
routine completion/cancellation banners are removed. Source/native failures stay
visible and can be dismissed. Releasing over this Sift process's windows (or no
valid window) cancels before a target drop, including winit's generic incoming
handler. The follow-up manual report reaches the handoff message; it does not
independently verify destination contents. The new native owned-window regression,
38 default-feature tests, actual Shell copying, formatting, and Clippy pass.

The integrated drag build's 60-second resource observation recorded about 82 MiB
private memory and no measurable CPU-time increase. Input events were recorded;
this is not a controlled idle qualification. See [baseline](BASELINE.md).

Collections is now the agreed product term. Its durable storage foundation is
implemented in a separate database with independent typed IDs, native paths,
bounded pages/queues, transactional edits, and explicit source availability.
Restart/index-rebuild preservation and unread-save shutdown tests pass. See
[collections architecture](COLLECTIONS.md). The desktop now creates/renames/deletes
collections, adds/removes source references, retains failed drafts for retry,
and loads bounded collection/content pages. Its compact table, upper waveform,
auto-play, optional details, and internal reference dragging are implemented.
47 desktop tests and 28 headless tests pass; explicit real-device concurrency
testing also passes. Pre-seeded native captures were visually checked, and
keyboard interaction created a collection in the actual Windows app. The
automation helper could not obtain pointer geometry, so physical mouse/drop
coverage remains explicit. Collection folders and last-active-view persistence
are not part of this first checkpoint.

The Collections release resource repeat measured about 82 MiB private memory
with no measurable CPU-time increase over 30 seconds. Activity diagnostics and
limits are recorded in [baseline](BASELINE.md); the longer run had substantial
input and cannot qualify idle CPU.

Next concrete step: verify bank creation, multiple source folders, subtree
selection, Collections switching, and the revised Explorer handoff on disposable
fixtures, including favorite toggling, restart, and Favorite-to-Explorer drops.
Then implement explicit durable-source relinking and profile the remaining
initial-indexing cost. Virtual collection folders, ordering, multi-selection, broader
device tests, and performance qualification remain later checkpoints.
Sift is still a prototype, not a finished sampler.

Choose major technical details from that evidence before introducing a full
analysis pipeline, AI, or a large application framework.
