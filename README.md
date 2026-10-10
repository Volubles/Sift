# Sift

Sift is a local sample browser for sound effects, samples, and music files.
It provides fast search, responsive audio previews, and collections for
collecting sounds and copying them into other folders. Low CPU and memory use
are core requirements.

The prototype provides a native Windows metadata browser and a CLI. It indexes
chosen folders into bundled SQLite/FTS5, searches bounded result pages, and
shows named libraries and a folder browser, a compact audio table, waveforms,
controlled audio preview, saved collections, global favorites,
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
**All sounds** restores the bank scope. **Ctrl+F** focuses search in the current
scope. Right-click a bank to rename/remove it or a source to rescan/remove its
registration. Source audio stays in place.

The folder tree separates opening a branch (chevron) from choosing its sounds
(name). Source folders have drive icons and cached counts/status; subfolders
have folder icons and hierarchy guides. Click the breadcrumb path above search
to return to a parent. In the focused tree, Up/Down move focus, Left/Right
close/open branches, Enter chooses the sound scope, and Home/End move to the
first/last loaded folder. Keyboard focus and the selected sound scope are distinct.

**Live updates** watches registered Windows folders automatically. Add, change,
rename, move, or remove sounds and the index follows after a brief settling
period. Pause with the checkbox; resuming checks for changes made while paused.
Unavailable roots retain cached data. Cancelling an update pauses watching.

Click a sound to inspect its metadata and waveform. Play/Pause,
Space, Stop, volume, waveform clicking, and **Auto-play** control the preview.
Choose the grid icon for **Collections**, preserving the current bank/folder/query.
Use **Collections +** to create a saved selection; right-click its name to rename
or delete it. **Add selected sound** or drag a library row onto a collection name
to add a reference. Click a saved sound to preview it; right-click to remove its
reference. Changes save automatically. **Details** toggles the metadata panel.
Click a sound's star in Libraries or Collections to favorite it. The star icon
in the left rail opens **Favorites** from all banks, with its own search and the
same preview/copy workflow. Favorite changes save automatically; removing a star
preserves collections and source audio. Missing favorites remain visible.
Hold the left
mouse button on a library row or **Drag copy to folder**, move into an Explorer
folder, and release to offer a copy. Escape cancels the drag. Explorer handles
file conflicts and copying progress. Completed/cancelled drags end quietly. Scanning can be
cancelled, and search runs independently from indexing. Selection is currently
single-item.

On Windows the catalog lives in `%LOCALAPPDATA%\Sift\catalog.sqlite`; it is
independent of the executable and working directory. Library registrations live
in `catalog.libraries.sqlite`; collections and favorites share
`catalog.collections.sqlite`. Both preserve user work through index rebuilds. Override with
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

See [Favorites](docs/FAVORITES.md) for global saved references, paging,
membership synchronization, and verification limits.
