# Sift

Sift is a local sample browser for sound effects, samples, and music files.
It aims to provide fast search, responsive audio previews, and projects for
collecting sounds and copying them into other folders. Low CPU and memory use
are core requirements.

The first checkpoint provides a Rust metadata catalog and CLI. It indexes chosen
folders into bundled SQLite/FTS5 and searches bounded result pages. The desktop
interface and playback are subsequent checkpoints; this is not a finished app.

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

## Build and use the catalog

Install Rust with rustup and the Visual Studio C++ build tools on Windows.
The repository pins Rust 1.98.1 and tracks Cargo.lock.

```powershell
cargo run --locked -- index "C:\path\to\your\samples"
cargo run --locked -- search "door wood"
cargo run --locked -- search "door" --offset 128
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
cargo fmt --all -- --check
```

The CLI defaults to `.local/sift.sqlite`; override it with `--database PATH`.
Indexing reads source files and commits metadata in batches of 64. It recognizes
WAV, FLAC, MP3, and AIFF, skips links/reparse points, and keeps per-file errors
visible. Queries return at most 128 records. Words of three or more characters
match literal substrings in names/relative paths; a shorter single query uses
an indexed filename prefix.

Current scans refresh metadata but do not reconcile deleted/moved files or skip
unchanged files yet. Projects, tags, playback, waveforms, and export are not
implemented. Do not treat indexed metadata as verified decoder/playback support.
