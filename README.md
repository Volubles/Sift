<h1 align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="assets/readme/sift-header-white.svg">
    <img src="assets/readme/sift-header-dark.svg" alt="Sift" width="360" height="129">
  </picture>
</h1>

<p align="center">
  A local sample browser for sound effects, samples, and music.<br>
  Find, preview, and collect sounds while keeping source files in place.
</p>

<p align="center">
  <a href="#features">Features</a> &middot;
  <a href="#getting-started">Getting started</a> &middot;
  <a href="#workflow">Workflow</a> &middot;
  <a href="#documentation">Documentation</a>
</p>

---

Sift is a **Windows prototype written in Rust**. It works offline, with local
storage and a focus on responsive browsing, bounded memory, and original audio.

![Sift's native library browser, showing folder navigation, waveform preview, and favorite sounds.](assets/screenshots/library-browser.png)

## Features

| <img src="assets/readme/browse.svg" width="28" height="28" alt=""> Browse | <img src="assets/readme/preview.svg" width="28" height="28" alt=""> Preview | <img src="assets/readme/collect.svg" width="28" height="28" alt=""> Collect |
| --- | --- | --- |
| Named libraries, multiple source folders, a folder tree, and filename/path search. | Waveforms, playback, seeking, volume, and auto-play for the selected sound. | Saved collections, global favorites, and copy-only dragging into Explorer. |

Registered folders update automatically when files are added, changed, renamed,
or removed. Incremental scans reuse unchanged metadata, and unavailable sources
keep their cached records and saved references.

Collections and favorites store **references**, without duplicating audio.
Their durable data survives rebuilding the search index.

## Getting started

Install [Rust with rustup](https://rust-lang.org/tools/install/) and the
[Microsoft C++ Build Tools](https://visualstudio.microsoft.com/visual-cpp-build-tools/)
with the **Desktop development with C++** workload on Windows.
The repository pins the toolchain and tracks `Cargo.lock`.

From the repository directory:

```powershell
cargo run --locked --release
```

1. Select **Libraries**, then **New** to create a library.
2. Use **Folders +** to register an existing audio folder.
3. Select a folder or search for a sound, then click a result to preview it.
4. Star the sound, save it in a collection, or drag a copy into an Explorer folder.

Audio stays in its source folder. Removing a library registration, collection
item, or favorite removes its reference, not the original file.

## Workflow

### Browse a library

Select a library for all its sounds, or a folder for that branch and its
descendants. The chevron opens subfolders; the folder name selects its sounds.
Clickable breadcrumbs return to ancestors, and **All sounds** restores the
library scope. **Ctrl+F** focuses search within the current scope.

### Preview a sound

Click a sound to load its waveform. Use **Play/Pause**, **Stop**, volume, or
**Auto-play**; click the waveform to play from that position. **Details** opens
source metadata and availability. Space toggles playback when focus is outside
text fields and the folder tree.

Auto-play starts enabled. Focus a sound list and use **W/S** or **Up/Down** to
audition neighboring sounds. **F** toggles the active sound's favorite.
**Ctrl+click**, **Shift+click**, and **Ctrl+A** mark sounds; see
[selection and keyboard navigation](docs/SELECTION.md) for scope and page behavior.

### Keep a selection

Star a sound to save it in **Favorites**, available across libraries and
collections. In **Collections**, use **Collections +** to create a selection,
then **Add selected sound** or an internal drag to add a reference. Changes save
automatically. A failed save remains visible for retry.

### Copy into your workflow

Drag a sound row out to an Explorer folder. Sift offers copying; Explorer handles
destination conflicts and progress. Escape or an unsuitable drop location cancels
the gesture. Original bytes and collection references stay in place.

### Keyboard shortcuts

| Context | Key | Action |
| --- | --- | --- |
| Library browser | Ctrl+F | Focus search |
| Folder tree | Up / Down | Move focus through loaded folders |
| Folder tree | Left / Right | Collapse / expand or move to a parent / child |
| Folder tree | Enter | Browse the focused folder's sounds |
| Folder tree | Home / End | Focus the first / last loaded folder |
| Preview | Space | Play / pause when other controls do not own the key |
| Sound list | W / S or Up / Down | Activate the previous / next sound |
| Sound list | Shift + navigation | Extend the marked range |
| Sound list | Ctrl+A | Mark all results in the current view |
| Sound list | F | Toggle the active sound's favorite |
| File drag | Escape | Cancel |

## Current scope

The index recognizes **WAV, FLAC, MP3, and AIFF**. Container metadata support does
not guarantee playback of every codec variant or compatibility with every device.

Collection adds/removals and outgoing dragging currently act on one sound.
Group actions, collection folders and item ordering, tags,
application-managed export, and explicit relinking of moved
saved sources are later work. Similarity search is optional future work.

Large-library measurements use generated fixtures with documented limits.
Initial import remains an optimization target; representative libraries, external
drives, NAS, and the broader audio-device matrix still need qualification.
See [performance measurements](docs/INDEXING-BASELINE.md) for the method and results.

## Local data

The Windows default is `%LOCALAPPDATA%\Sift\`. Data lives independently of the
executable and working directory.

| File | Purpose |
| --- | --- |
| `catalog.sqlite` | Rebuildable search index |
| `catalog.libraries.sqlite` | Durable library and source-folder registrations |
| `catalog.collections.sqlite` | Durable collections and favorites |
| `catalog.waveforms.sqlite` | Rebuildable waveform cache |

`--database PATH` selects a separate catalog for experiments. The library and
collection databases hold durable data independently of the rebuildable index/cache.

## Development

The application uses Rust, egui/eframe with Glow, bundled SQLite/FTS5, Symphonia,
and CPAL. Presentation, storage, decoding, and native Windows integration have
separate owners; background queues and caches are bounded. See
[architecture](docs/ARCHITECTURE.md) for responsibilities and thread boundaries.

```powershell
cargo fmt --all -- --check
cargo check --locked
cargo clippy --locked --all-targets -- -D warnings
```

Automated tests, fixtures, and measurement helpers are maintained privately and
are not included in this source distribution. See the linked documentation for
recorded verification and remaining manual checks. Private libraries, databases,
exports, build output, and working notes stay outside version control.

<details>
<summary>Command-line indexing and search</summary>

```powershell
cargo run --locked --no-default-features -- index "C:\path\to\samples"
cargo run --locked --no-default-features -- search "door wood"
cargo run --locked --no-default-features -- search "door" --offset 128
```

The CLI configuration shares the same catalog and durable data as the desktop.
Use `--database PATH` to isolate a test. Queries return at most 128 results per page.

</details>

## Documentation

| Topic | Read |
| --- | --- |
| Browser and saved sounds | [Libraries](docs/LIBRARIES.md), [Collections](docs/COLLECTIONS.md), [Favorites](docs/FAVORITES.md) |
| Audio and file transfer | [Preview](docs/PREVIEW.md), [Drag and drop](docs/DRAG-AND-DROP.md) |
| Marking and keyboard controls | [Selection](docs/SELECTION.md) |
| Files and live updates | [Incremental indexing](docs/INDEXING.md) |
| Design and behavior | [Architecture](docs/ARCHITECTURE.md), [Product overview](docs/PRODUCT.md) |
| Resource use and evidence | [Performance](docs/PERFORMANCE.md), [Renderer baseline](docs/BASELINE.md), [Index measurements](docs/INDEXING-BASELINE.md) |
| Brand assets | [SVG masters and transparent PNG exports](assets/brand/README.md) |
