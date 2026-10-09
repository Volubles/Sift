# Sift

Sift is a local sample browser for sound effects, samples, and music files.
It aims to provide fast search, responsive audio previews, and projects for
collecting sounds and copying them into other folders. Low CPU and memory use
are core requirements.

The prototype provides a native Windows metadata browser and a CLI. It indexes
chosen folders into bundled SQLite/FTS5, searches bounded result pages, and
shows audio metadata. Playback and projects are later checkpoints; this is not
a finished application.

Development uses small, coherent Conventional Commits at natural checkpoints.
Private audio libraries, databases, exports, raw measurements, and IDE settings
do not belong in the repository.

All project documentation and user-visible content are written in English.

## Start here

- [AGENTS.md](AGENTS.md): working agreement, code quality, and verification.
- [Current status](docs/STATUS.md): what exists and the next step.
- [Product requirements](docs/PRODUCT.md): features, invariants, and acceptance.
- [Architecture](docs/ARCHITECTURE.md): technical direction and responsibilities.
- [Stack review](docs/STACK-REVIEW.md): alternatives, evidence, and prototype gates.
- [Performance](docs/PERFORMANCE.md): provisional budgets and measurement method.
- [Git conventions](GIT-COMMIT-CONVENTIONS.md): small, natural checkpoints.

## Run the prototype

Install Rust with rustup and the Visual Studio C++ build tools on Windows.
The repository pins Rust 1.98.1 and tracks Cargo.lock.

```powershell
cargo run --locked
cargo run --locked -- index "C:\path\to\your\samples"
cargo run --locked -- search "door wood"
cargo run --locked -- search "door" --offset 128
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
cargo fmt --all -- --check
```

The desktop opens by default. Use **Add folder** to choose a library; **Ctrl+F**
focuses search. Click a sound to inspect its metadata. Scanning can be cancelled,
and search runs independently from indexing. Selection is currently single-item.

On Windows the catalog lives in `%LOCALAPPDATA%\Sift\catalog.sqlite`; it is
independent of the executable and working directory. Override with
`--database PATH` for isolated experiments. The CLI-only configuration is
available through `cargo run --locked --no-default-features -- index FOLDER`.

Indexing reads source files and commits metadata in batches of 64. It recognizes
WAV, FLAC, MP3, and AIFF, skips links/reparse points, and keeps per-file errors
visible. Queries return at most 128 records. Words of three or more characters
match literal substrings in names/relative paths; a shorter single query uses
an indexed filename prefix.

The view renders visible rows and caches at most four 128-record pages. Workers
use bounded queues and wake the UI on results; there is no idle polling timer.
The prototype uses egui/eframe 0.36.2, the Glow renderer, and RFD 0.17.2 for the
Windows folder dialog. See [checkpoint verification](docs/CHECKPOINT.md).

Current scans refresh metadata but do not reconcile deleted/moved files or skip
unchanged files yet. Projects, tags, playback, waveforms, and export are not
implemented. Do not treat indexed metadata as verified decoder/playback support.
