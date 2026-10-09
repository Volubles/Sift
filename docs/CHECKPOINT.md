# Metadata browser checkpoint

Verified on Windows, 2026-10-09. This is an initial usable slice, not completion
of the product roadmap or the full technical prototype acceptance list.

## Implemented

- Native dark desktop window with folder selection, search, single selection,
  metadata details, scan progress/issues, and cancellation.
- Persistent SQLite/FTS5 catalog, 64-record scan transactions, and 128-record
  query pages. Four pages maximum in the view, plus the selected record.
- Separate index/search workers, bounded channels, stale-query rejection,
  on-demand repaint, and shutdown that releases blocked channel producers.
- Lossless native paths, source-preserving metadata reads, and explicit errors.
- Rust 1.98.1, locked dependencies, optional desktop feature, and CLI commands.

## Evidence

- `cargo test --locked`: 15 tests passed, including rollback, cancellation,
  stable paging, simultaneous visible pages, full-queue shutdown, and scroll-back
  cache retention.
- `cargo test --locked --no-default-features`: 14 core/worker tests passed.
- `cargo clippy --locked --all-targets -- -D warnings`: passed.
- `cargo fmt --all -- --check`: passed.
- `cargo build --locked`: runnable native executable built in the dev profile.
- Generated WAV, FLAC, MP3, and AIFF files were indexed without probe errors.
  SHA-256 comparison showed unchanged sources. This proves metadata probing
  for those fixtures, not general decoder or playback support.
- An actual WGPU Windows window rendered the four-fixture catalog and a separate
  100,000-record synthetic catalog. The latter loaded 128 records into the view.
  Pixel captures were read back from the native renderer and visually inspected.
- Scanner/read-worker integration is exercised through the `browse --folder`
  entry point against disposable generated fixtures.

Captures, databases, generated sounds, and the synthetic-data helper are private
scratch artifacts under ignored `.local/`. No private sample collection was
scanned. The diagnostic capture option writes a new PPM file, closes the window,
and reports failure if capture does not complete. It is not a normal idle loop.

## Current limits

- No preview playback, waveform, projects, manual tags, export, or similarity.
- No move/deletion reconciliation or incremental skip of unchanged files.
- No arbitrary library-size promise, release CPU/RAM/latency measurements,
  audio-device tests, native outgoing dragging, or tested non-Windows UI support.
- The native folder picker is wired to RFD; interactive picker selection,
  keyboard focus, and manual scrolling still need a hands-on session.
- Source probe reads are capped at 8 MiB; visuals are skipped and individual
  metadata tags are limited to 4 KiB. Large unusual headers may report errors.
- Cancellation is cooperative; an OS read blocked on an unavailable network
  filesystem cannot be interrupted by the current token.
- The viewport uses indexed metadata snapshots rather than live filesystem
  status. Missing or modified files require subsequent reconciliation work.

## Next checkpoint

Add waveform analysis and controlled preview playback using disposable audio:
bounded decode, rate conversion, callback transport, fast cancellation/seek,
and device recovery. Measure real resource use before expanding the scope.
