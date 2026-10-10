# Sift

Sift is a local sample browser for sound effects, samples, and music files.
It provides fast search, responsive audio previews, and collections for
collecting sounds and copying them into other folders. Low CPU and memory use
are core requirements.

The prototype provides a native Windows metadata browser and a CLI. It indexes
chosen folders into bundled SQLite/FTS5, searches bounded result pages, and
shows named libraries and a folder browser, a compact audio table, waveforms,
controlled audio preview, saved collections,
and outgoing copy-only file dragging on Windows. This is not a finished
application.

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

The desktop opens by default. Choose the drive icon for **Libraries**, use **New**
to create a bank, and **Add source folder...** to register existing directories.
Choose a bank for all its sounds, or a folder for that branch and its descendants.
**All files** restores the bank scope. **Ctrl+F** focuses search in the current
scope. Right-click a bank to rename/remove it or a source to rescan/remove its
registration. Source audio stays in place.

Click a sound to inspect its metadata and waveform. Play/Pause,
Space, Stop, volume, waveform clicking, and **Auto-play** control the preview.
Choose the grid icon for **Collections**, preserving the current bank/folder/query.
Use **Collections +** to create a saved selection; right-click its name to rename
or delete it. **Add selected sound** or drag a library row onto a collection name
to add a reference. Click a saved sound to preview it; right-click to remove its
reference. Changes save automatically. **Details** toggles the metadata panel.
Hold the left
mouse button on a library row or **Drag copy to folder**, move into an Explorer
folder, and release to offer a copy. Escape cancels the drag. Explorer handles
file conflicts and copying progress. Completed/cancelled drags end quietly. Scanning can be
cancelled, and search runs independently from indexing. Selection is currently
single-item.

On Windows the catalog lives in `%LOCALAPPDATA%\Sift\catalog.sqlite`; it is
independent of the executable and working directory. Library names/sources and
collections are stored separately in `catalog.libraries.sqlite` and
`catalog.collections.sqlite`, preserving user work through index
rebuilds. Override with
`--database PATH` for isolated experiments. The CLI-only configuration is
available through `cargo run --locked --no-default-features -- index FOLDER`.

Indexing reads source files and commits metadata in batches of 64. It recognizes
WAV, FLAC, MP3, and AIFF, skips links/reparse points, and keeps per-file errors
visible. Queries return at most 128 records. Words of three or more characters
match literal substrings in names/relative paths; a shorter single query uses
an indexed filename prefix.

The view renders visible rows and caches at most four 128-record sound pages.
Folder browsing uses 64-row pages with up to 32 open branches. Workers
use bounded queues and wake the UI on results; there is no idle polling timer.
The prototype uses egui/eframe 0.36.2, the Glow renderer, and RFD 0.17.2 for the
Windows folder dialog. See [checkpoint verification](docs/CHECKPOINT.md).

Scans skip unchanged metadata and reconcile missing index rows only after a
complete inventory; cancellation and inaccessible sources preserve cached data.
See [Indexing](docs/INDEXING.md). Collection folders/reordering, tags, multi-selection, and application-managed
folder export are not implemented. Do not treat indexed metadata as verified
decoder/playback support.

See [preview verification](docs/PREVIEW.md) for tested fixtures, device behavior,
cache limits, and remaining checks. Source files remain unchanged.

See [native drag verification](docs/DRAG-AND-DROP.md) for transfer limits,
actual Windows Shell copying evidence, and the remaining manual Explorer checks.

See [Collections](docs/COLLECTIONS.md) for durable storage, source availability,
save-failure recovery, UI verification, and remaining checks.

See [Libraries](docs/LIBRARIES.md) for named banks, native folder scopes, schema
migration, source registration limits, and verification.
