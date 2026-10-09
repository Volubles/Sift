# Current status and next steps

Last updated: 2026-10-09. This document describes actual deliveries; product
and architecture documents describe the intended application.

## Completed

- Local Git repository on `main`, repository hygiene, Conventional Commit workflow.
- AGENTS.md with durable quality requirements and instructions for resuming work.
- Requirements for libraries, search, preview, projects, and copying.
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
on Windows. Projects, tags, multi-selection, and application-managed export are
not implemented yet. No CI
configuration exists yet. Active implementation branch: `codex/library-prototype`;
check Git before assuming a branch in a future session.
No remote, publishing, or distribution license has been selected.

## Decisions and open choices

- Agreed: Rust, efficient local workflow, projects as references, copy-only
  outgoing drag-and-drop, English artifacts, and small checkpoint commits.
- Reviewed prototype direction: Windows first, egui/eframe, bundled SQLite/FTS5
  through rusqlite, Symphonia, CPAL, Rubato when needed, rtrb, and a limited
  windows-rs adapter. Slint is the main UI alternative. Validate candidates
  before selecting dependency versions or claiming comparative performance.
- Open: visual design, exact format matrix, device/backend support, approved
  sample collection, final performance budgets, and distribution/license.
- Similarity search is later and optional; AI categorization is outside the core.

## Verification

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

Verify the manual Explorer flow, then implement durable project references,
ordering/folders, and multi-selection. Keep incremental indexing,
source reconciliation, broader device tests, and performance qualification
explicit. Sift is still a prototype, not a finished sampler.

Choose major technical details from that evidence before introducing a full
analysis pipeline, AI, or a large application framework.
