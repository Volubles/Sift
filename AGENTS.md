# Sift: agent working agreement

Sift is a local, resource-efficient sample browser written in Rust. Its core
workflow is finding sounds, previewing them, saving references in collections, and
copying selections to other folders. Preserve source files and user data.
Low CPU/RAM usage, responsiveness, and readable code are quality requirements.

## Resume work

For a new chat or substantial task, read [current status](docs/STATUS.md),
check the branch, `git status --short --branch`, and relevant history. Distinguish
planned architecture from existing implementation. Inspect the affected flow
and callers before changing it; do not assume an earlier chat proved it works.
For a small text correction, read the relevant file and applicable instructions.

Use the document that matches the task:

| Task | Read |
| --- | --- |
| Features, behavior, or scope | [Product requirements](docs/PRODUCT.md) |
| Boundaries, storage, threads, or native integration | [Architecture](docs/ARCHITECTURE.md) |
| Choosing or replacing a framework/backend | [Stack review](docs/STACK-REVIEW.md) |
| Resource cost, large libraries, or optimization | [Performance](docs/PERFORMANCE.md) |
| Branches, staging, and checkpoints | [Git conventions](GIT-COMMIT-CONVENTIONS.md) |

## Collaboration and scope

- Communicate with Benjamin in Swedish. All repository documentation, README
  content, product UI, messages, comments, identifiers, tests, and commits must
  be in clear English. Swedish belongs in the conversation, not public artifacts.
- Own investigation, implementation, and verification within the requested task.
  Continue to a complete, reviewable result. Do not request renewed approval for
  authorized local changes, relevant checks, or checkpoint commits.
- Briefly explain the impact before substantial changes. Ask only when a missing
  product decision or value prevents a correct next step.
- A documentation request authorizes completing the documentation; it does not
  automatically authorize starting the next implementation phase.
- Preserve unrelated changes. Do not mix unrelated cleanup or new features into
  a task unless they are necessary to complete it.
- Pushing, publishing, releases, and messages to other people require explicit
  user instructions. Do not create a remote or choose a public license by habit.
- Scan only libraries identified for the task or approved fixtures. Do not
  delete, move, or overwrite private originals as a testing technique.
- Do not start parallel agents unless the user requests them. When delegated,
  bound ownership; the main agent owns integration, verification, and commits.

## Product invariants

- Collection items are references. Tagging, collection changes, and export do not alter
  source names, contents, or locations. External drag-and-drop offers copying.
- Separate durable collections/tags/favorites from rebuildable indexes and caches.
  Reindexing and cache cleanup must preserve the user's work.
- Missing sources, unknown metadata, analysis errors, and pending jobs are
  distinct states. Do not silently resolve ambiguous file matches.
- The core workflow works offline without AI categorization. Similarity search
  must not be required to open, search, or use ordinary collections.
- Silent replacement, reporting partial export as complete success, and losing
  unsaved collection edits are bugs even when the normal path works.

## Code quality and ownership

- Write idiomatic, readable Rust with descriptive names, straightforward control
  flow, cohesive modules, and functions with a clear responsibility.
- Fix the root cause at the smallest boundary that owns the rule. Inspect all
  callers when shared behavior changes; do not duplicate special cases in UI.
- Start with concrete types. Add traits/generics for a real boundary, testing
  need, or reuse requirement. Avoid empty layers, universal managers,
  speculative plugin frameworks, and a crate per module.
- Test domain rules without UI, audio hardware, or Windows. UI presents state;
  SQL, decoding, and file copying belong in their owning modules.
- Keep APIs small and visibility intentional. Use distinct ID types, enums for
  states, and `Option` for unknown values. Avoid unclear boolean parameters.
- Make ownership and lifetimes explicit. Avoid global mutable state, unnecessary
  `Arc<Mutex<_>>`, hidden initialization requirements, and clones that disguise
  ownership problems.
- Isolate necessary `unsafe` at native/FFI boundaries. Document the safety
  invariant and review pointer lifetime, aliasing, and thread affinity.
  Do not use unsafe for unproven performance gains.
- Use `Result` for fallible operations. `unwrap`, `expect`, `panic!`, and
  `unreachable!` must not replace handling of files, user input, SQL, or devices.
  Tests and proven internal invariants may have narrowly justified exceptions.
- Do not discard errors with `let _ =`, empty branches, or successful defaults.
  Best-effort cleanup must still report failure at the owning error boundary.
- Comments explain invariants and decisions, rather than restating code.
  Temporary TODOs need concrete remaining work. Do not deliver core behavior
  as a placeholder or `todo!()`.

## Threads, audio, and resource limits

- No disk I/O, SQL, hashing, or lengthy decoding/analysis on the UI thread.
- No I/O, logging, heap allocation, or blocking synchronization in the audio
  callback. Prepare audio outside the callback with bounded buffering.
- Bound queues, threads, connections, and caches deliberately. Define ownership,
  invalidation, cancellation, and shutdown for background work.
- Prioritize interaction and playback over analysis. Use job IDs/generations so
  stale responses cannot update a newer selection or query.
- Render visible rows and repaint when needed. Avoid continuous redraw, tight
  timers, and full scans while idle.
- Do not load the library's audio into RAM. Long files and large batches require
  bounded memory. Caches need size limits and measurable value.
- Rust or a new dependency does not prove performance. Profile actual cost and
  report before/after evidence when claiming an improvement.

## Storage, files, and errors

- Validate external boundaries: files, metadata, search input, configuration,
  serialized collections, and native responses. Parameterize SQL.
- Use versioned schemas and transactions for durable changes. Preserve data on
  migration failure; unsupported versions must not reset the database.
- File identity, location, and content hash are different concepts. Preserve OS
  paths without loss; display strings must not become file-operation locators.
- Export names must handle traversal, reserved names, collisions, and targets
  aliasing sources. Account for links/reparse points and relevant write races.
- Plan conflicts, create unique temporary files, and verify actual publication
  semantics. Copy original bytes; do not transcode without a request.
- Log unexpected failure once at its handling boundary: operation, stage,
  relevant ID, cause chain, and outcome. Present an actionable UI message.
- Normal cancellation/validation does not require an error stack. Bound repeated
  diagnostics; no hot-path logging or private payloads.
- Keep logs and diagnostics local and bounded. Shared reports must not contain
  private library names, personal absolute paths, or secrets.

## UI and dependencies

- Make lists, multi-selection, keyboard focus, playback, and collection state clear.
  Show progress, missing sources, and save failures without freezing the UI.
- Preserve selection and relevant view settings through updates. Internal
  dragging is not proof of support for external applications.
- Choose maintained, stable tools for concrete needs. Check versions, platform
  support, licenses, feature flags, and actual integration cost.
- Pin the toolchain and application dependency graph when Cargo is introduced.
  Start with minimal features/dependencies; do not upgrade everything in a fix.
- Use Context7 when available for library-specific documentation; verify against
  pinned versions and official docs or source. Documentation is not proof of
  local runtime compatibility.
- Introduce an async runtime, service, network requirement, or new UI stack only
  for a concrete need and a documented decision.

## Verification and completion

Choose checks that prove the changed behavior. Relevant tests with disposable
fixtures can run within the task without asking again. Code changes require
formatting, relevant tests, and lint for affected targets to pass. Preserve exit
codes; the final command's success must not hide an earlier failure.

Once a Cargo project and lockfile exist, standard commands are:

```powershell
cargo fmt --all -- --check
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
```

Adapt test selection/features to actual code and documented CI. Do not run these
in a documentation repository without `Cargo.toml`. Do not build for branch or
text changes, or repeat expensive checks without a new reason.

- Test domain rules, search semantics, persistence, and migrations with small,
  genuine fixtures.
- Audio/files: format variants, long files, seeking, cancellation, missing or
  modified sources, and malformed data. Use small fixtures with known rights.
- Collections/export: restart, rollback, conflicts, partial failures, and unchanged
  originals. Verify reindexing preserves manually entered user data.
- Concurrency: stale responses, rapid selection, shutdown during work, and queue
  bounds. Prefer controlled coordination over arbitrary timing sleeps.
- Native/audio: verify playback and dragging into Explorer on actual Windows.
  Mocks, builds, and screenshots do not prove these interactions.
- Performance: follow the scenarios and reporting format in the performance doc.
- Documentation: check facts, local links, and staged changes. Do not write
  implementation-mirroring tests for a low-impact text correction.

Complete means the requested behavior is implemented or documentation delivered,
relevant checks have the expected outcome, error/cancellation cases are handled,
new responsibilities/decisions are documented, and a natural checkpoint is
committed. If runtime proof is missing, name the exact remaining check and limit.
Never claim tests, builds, playback, or performance passed without evidence.

## Commits and handover

Create small Conventional Commits at natural, verified checkpoints without
waiting for a separate command. Each checkpoint must be coherent and meaningful;
do not combine a whole version into one commit or manufacture a commit per file.
Stage explicit paths and inspect the staged diff. Preserve unrelated work and
history. See the Git conventions for details.

Update `docs/STATUS.md` at a substantial completed step or before handover, as
part of the relevant commit. Record what exists, evidence, open decisions,
blockers, and the next concrete step. Use dates/descriptions; avoid
self-referential commit hashes or a separate status commit after every edit.

The final report should briefly state the result, relevant files, branch,
new commit hashes and subjects, verification, and pending runtime checks.
Tell Benjamin what to do only when an actual action is needed.

Track AGENTS.md and project documentation in Sift and keep them current.
Place general rules here and detailed specifications in linked documents.
Do not turn this file into a repeated chat log or copy unrelated project rules.
