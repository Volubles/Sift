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

The desktop UI, playback, projects, and export are not implemented yet. No CI
configuration exists yet.
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

The core compiles; eleven tests and warning-free Clippy pass. Formatting is
checked. Playback, native dragging, and application performance have not been
measured; metadata support must not be presented as playback proof.
The CLI also indexed generated WAV, FLAC, MP3, and AIFF fixtures without errors;
all four SHA-256 source hashes were unchanged afterward.

## Next proposed task

Add the small egui desktop shell with independent index/search workers, a virtual
list, folder selection, cancellation, and metadata inspection. Then prove
waveform/preview and native dragging in separate coherent checkpoints. Use
disposable fixtures and measure idle resources, latency, and memory.

Choose major technical details from that evidence before introducing a full
analysis pipeline, AI, or a large application framework.
