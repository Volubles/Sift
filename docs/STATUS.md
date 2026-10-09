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

No `Cargo.toml`, Rust implementation, built application, or CI exists yet.
No remote, publishing, or distribution license has been selected.

## Decisions and open choices

- Agreed: Rust, efficient local workflow, projects as references, copy-only
  outgoing drag-and-drop, English artifacts, and small checkpoint commits.
- Initial prototype direction: Windows first, egui/eframe, SQLite/FTS5,
  Symphonia, CPAL, and a limited Windows adapter. Stack research is in progress;
  validate candidates before selecting dependency versions.
- Open: visual design, exact format matrix, device/backend support, approved
  sample collection, final performance budgets, and distribution/license.
- Similarity search is later and optional; AI categorization is outside the core.

## Verification

The repository foundation was checked for UTF-8/line endings, local links,
ignore behavior, and text/binary attributes. Review staged diffs at each
checkpoint. Git history is the source of current hashes and branch.

No application compilation, playback, native dragging, or performance
measurement has been performed. Specifications are not runtime proof.

## Next proposed task

Finish the stack comparison before implementation. Once implementation is
requested, create a minimal Cargo project with a compatible pinned stable
toolchain. Prove a large virtual list, waveform, preview, and native dragging
into Explorer. Use disposable fixtures, natural checkpoints, and measurements
of idle resources, latency, and memory.

Choose major technical details from that evidence before introducing a full
analysis pipeline, AI, or a large application framework.
