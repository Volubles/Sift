# Stack review

Reviewed: 2026-10-09. Evidence: official documentation, release-tag source,
upstream repositories, and current crate registry metadata. No candidate has
been compiled, benchmarked, or exercised with Windows audio/dragging in Sift.
Recommendations below are engineering judgments, not a measured ranking.

## Recommendation

Keep Rust, egui/eframe, SQLite/FTS5 through rusqlite, Symphonia, and CPAL as the
first prototype direction. Make resampling and bounded audio transport explicit:
evaluate Rubato and rtrb. Use a small windows-rs adapter for outgoing copy-only
file dragging. Slint is the main UI alternative if the prototype reveals a
meaningful design, resource, or integration disadvantage in egui.

The backend runs in the desktop process. UI communicates through typed commands
and results with bounded worker queues. No HTTP server, local web service,
separate database server, or mandatory async runtime is needed for this scope.

"Best" means meeting Sift's measured requirements with acceptable complexity,
reliability, and maintainability. Newer implementation languages, GPU rendering,
or vendor benchmarks do not establish that result for this workload.

## UI comparison

| Candidate | Relevant evidence | Assessment for Sift |
| --- | --- | --- |
| [egui/eframe](https://github.com/emilk/egui) | Portable native Rust integration, custom painting, visible-row rendering, repaint on demand; MIT/Apache licensing. | First candidate for a dense library tool and waveforms. Outgoing native file dragging remains an explicit adapter/prototype task. |
| [Slint](https://github.com/slint-ui/slint/tree/v1.18.1) | Declarative UI and virtualized ListView; released native outgoing drag support is in its Qt backend. | Strong design-oriented alternative. Compare an actual renderer/backend configuration, including its distribution footprint. |
| [Iced](https://github.com/iced-rs/iced/tree/0.14.0) | Reactive Rust UI with explicit state/messages and incoming file events. | Credible alternative, but the reviewed public API did not establish an outgoing Explorer file-drag advantage. Moving a window is not file export. |
| [GPUI](https://github.com/zed-industries/zed/blob/main/crates/gpui/README.md) | GPU framework used by Zed; current source supports Windows, but explicitly warns of pre-1.0 breaking changes. | Attractive for a highly custom app, with additional integration/API churn. Distinguish published crates from current Zed source; no resource advantage was demonstrated. |
| [Tauri](https://v2.tauri.app/reference/webview-versions/) | Rust backend with a system webview; Windows uses WebView2. | Useful when web UI development is the priority. Not the first candidate for this resource-focused native tool. No local comparison was performed. |
| [C++/JUCE](https://docs.juce.com/master/classjuce_1_1DragAndDropContainer.html) | A native external-file drag API already offers a copy-only choice. | A serious alternative when integrated audio-tool/plugin infrastructure dominates. That API is an integration advantage, not proof of lower resource use or a reason by itself to replace the agreed Rust core. |

### Slint's important backend distinction

The [1.18.0 release notes](https://github.com/slint-ui/slint/blob/v1.18.1/CHANGELOG.md#1180---2026-09-16)
specifically add native outgoing drag to Qt. The
[1.18.1 notes](https://github.com/slint-ui/slint/blob/v1.18.1/CHANGELOG.md#1181---2026-09-21)
fix a transfer containing only file paths. These are relevant released changes,
not promises about every Slint backend. The default
[window adapter](https://github.com/slint-ui/slint/blob/v1.18.1/internal/core/window.rs)
can decline native dragging, and no override was found in the reviewed
[winit window adapter](https://github.com/slint-ui/slint/blob/v1.18.1/internal/backends/winit/winitwindowadapter.rs).

Do not select Slint/winit on the assumption that Qt's outgoing integration comes
with it. A Qt deployment changes dependencies and packaging. Slint's
[royalty-free desktop license](https://github.com/slint-ui/slint/blob/v1.18.1/LICENSES/LicenseRef-Slint-Royalty-free-2.0.md)
permits desktop distribution with attribution conditions; GPL/commercial routes
are separate choices. Record the actual chosen route if Slint is adopted.

Slint's [ListView](https://github.com/slint-ui/slint/blob/v1.18.1/docs/astro/src/content/docs/reference/std-widgets/views/listview.mdx)
creates visible elements, which is useful for large lists. egui also supports
[visible-row rendering](https://docs.rs/egui/latest/egui/containers/scroll_area/struct.ScrollArea.html#method.show_rows)
and on-demand repaint. Neither mechanism proves the application's memory or
latency budget without measurement.

## Storage and search

| Candidate | Relevant evidence | Assessment |
| --- | --- | --- |
| [SQLite + rusqlite](https://github.com/rusqlite/rusqlite) | Embedded transactions, SQL, FTS5; rusqlite can bundle SQLite for consistent Windows deployment. | Preferred fit for project relationships, metadata filters, and indexed text search. Keep blocking operations off UI/audio threads. |
| [SQLx with SQLite](https://github.com/transact-rs/sqlx) | Async APIs, pooling, optional compile-time query checks; SQLite still uses its native C library. | Useful with broader async/database needs. Its API adds no demonstrated benefit to this small local worker model. |
| [Turso Database](https://github.com/tursodatabase/turso) | Current Rust engine reports production use, SQLite compatibility, vector operations, and concurrency improvements; full-text search and some other features are explicitly experimental. | A genuine modern alternative. Retain SQLite because established FTS5 semantics matter now; reconsider only with Sift-specific search/compatibility and recovery evidence. |
| [redb](https://github.com/cberner/redb) | Stable, transactional Rust key-value store. | Good for key/value access; Sift would have to build more relational/query/text-index behavior itself. Its KV benchmarks do not measure Sift's search workload. |
| [DuckDB](https://github.com/duckdb/duckdb) | An embedded analytical SQL engine. | Useful for later bulk analytics, not an identified improvement for interactive project writes and sample search. |

Specify search semantics before building the schema. Ordinary token search,
prefix search, and substring search are different requirements. Evaluate FTS5
tokenization for names such as `door_wood_001.wav`; its
[trigram tokenizer](https://sqlite.org/fts5.html#the_trigram_tokenizer)
supports substring queries but has short-query limitations and index costs.
Define and benchmark one/two-character queries, punctuation, Unicode, tag
matching, literal quoting, and stable result ordering. Do not quietly fall back
to a full-library scan on every keystroke.

## Audio and streaming

| Candidate | Responsibility and judgment |
| --- | --- |
| [Symphonia](https://github.com/pdeljanov/Symphonia) | Decode and extract metadata. Keep the actual codec/bit-depth/seek matrix explicit. |
| [CPAL](https://docs.rs/cpal/0.18.2/cpal/) | Device I/O. Begin by evaluating Windows WASAPI shared operation so preview can coexist with other applications. Verify actual device configuration and recovery. |
| [Rubato](https://github.com/HEnquist/rubato) | Candidate for explicit sample-rate conversion. It offers quality/speed tradeoffs and chunk processing. Equal-rate playback should bypass unnecessary resampling. |
| [rtrb](https://github.com/mgeier/rtrb) | Candidate for preallocated, bounded SPSC transport from decode worker to audio callback. Its wait-free transport does not make arbitrary callback code safe. |
| [Rodio](https://docs.rs/rodio/0.22.2/rodio/) | Higher-level playback using Symphonia/CPAL, including playback controls and seeking. Keep as a simpler baseline; verify buffering, cancellation, decoding location, and latency before replacing the controlled pipeline. |
| [Creek](https://docs.rs/creek/1.2.3/creek/) | Real-time-safe disk-streaming reader with a Symphonia adapter. Evaluate if it reduces custom streaming work; check decoder-version compatibility and seek/cancellation behavior first. |

CPAL is device I/O, not a complete player. Sift still owns decoding, resampling,
channel mapping, volume, seek, buffering, stale-job rejection, and shutdown.
The first prototype must test mixed source sample rates and channel layouts,
long compressed files, rapid selection, and underruns during background work.
Do not enable ASIO or introduce a DSP graph unless the requested workflow needs it.

## Windows integration and development tools

Use [windows-rs](https://github.com/microsoft/windows-rs) for a narrow native
boundary. [DoDragDrop](https://learn.microsoft.com/en-us/windows/win32/api/ole2/nf-ole2-dodragdrop)
offers an allowed-effect mask; advertise copying only. Verify COM apartment
ownership, file payloads, cancellation, and the drag event loop with actual
Explorer. Document target-specific DAW behavior separately.

Keep Cargo, rustfmt, and Clippy as the build/check foundation. Evaluate
[cargo-deny](https://embarkstudios.github.io/cargo-deny/) once the dependency graph
exists, with intentional advisory/license/source policy. Consider
[cargo-nextest](https://nexte.st/) as the test suite grows. These are development
tools, not product runtime dependencies; no installation or CI setup occurred
during this review.

## Observed release snapshot

Crates.io's API was queried on the review date. These are available stable
versions, not compatibility-tested pins or approval to install everything:

| Crate | Observed stable version |
| --- | --- |
| egui | 0.36.2 |
| Slint | 1.18.1 |
| Iced | 0.14.0 |
| GPUI | 0.2.2 |
| rusqlite | 0.40.2 |
| Symphonia | 0.6.1 |
| CPAL | 0.18.2 |
| Rubato | 5.0.1 |
| Rodio | 0.22.2 |
| Creek | 1.2.3 |

Recheck releases, matching framework crates, minimum Rust versions, and
cross-crate compatibility at bootstrap. Pin a verified compatible set rather
than selecting every latest version independently.

## Prototype acceptance before final stack selection

1. Implement one thin egui shell against disposable data. Prove multi-selection,
   keyboard interaction, a virtual 100,000-row view, waveform, and outgoing drag.
2. Exercise decoding/preview at different rates and channel counts, plus seeking
   in long files, device interruption, cancellation, and background interference.
3. Probe bundled SQLite/FTS5 availability and actual search semantics/query plans;
   test transactions, restart, and preservation of manually entered data.
4. Measure startup, idle private memory/CPU, scrolling, audible latency, and
   search against the [proposed budgets](PERFORMANCE.md). Track driver/GPU cost.
5. If egui misses a product requirement or measured budget, compare a small
   Slint shell using the same workload and record renderer/backend settings.
6. Record results and a final decision in [architecture](ARCHITECTURE.md).
   A successful build is necessary but does not establish runtime acceptance.
