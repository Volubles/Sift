# Named libraries and folder browsing

A library is a named bank of existing source folders. Collections are saved
sound references. Registering, renaming, or removing either never moves or
deletes source audio. A bank may span disks and contain overlapping folders;
the catalogue stores each exact native file locator once.

## Storage foundation

Library names and source registrations live in a separate schema-1 database,
`catalog.libraries.sqlite` beside the selected catalogue. Typed, non-reused
library/source IDs, lossless native paths, foreign keys, transactions, and full
SQLite synchronization protect accepted changes. An index rebuild can leave
this file and the collection database intact. The prototype bounds registrations
to 128 banks, 32 sources per bank, and 512 sources overall.

Catalog schema 2 adds a global native-locator index and a derived folder table.
The transactional migration keeps the oldest ID for duplicate locations,
updates FTS through its existing triggers, and builds the folder tree in
128-record batches. Invalid locators abort migration without advancing its
version. Unknown schema versions are rejected without resetting data.

Folder queries use separator-terminated native-byte ranges on the source index,
including descendants without matching similarly named sibling folders. Nested
and repeated roots are coalesced. Search and count share a transaction snapshot;
folders have bounded 64-row child pages. Display strings never locate files.
Folders with indexed audio form the tree; empty directories are omitted.

The header now shows a clickable native-path breadcrumb trail for the current
bank/source/subfolder. Selecting an ancestor preserves the bank and search text.
Deep paths collapse middle labels into an accessible menu while keeping their
native targets. A single source named like its bank does not repeat that label.
Revealing ancestors is bounded by the existing open-branch limit and performs
no filesystem inspection. Tests cover Unicode targets, overlapping sources,
deep paths, path validation, and clicking the trail back to the bank scope.

Previous catalogue roots can be imported once without opening or rescanning
their files. An import marker prevents removed banks from reappearing on restart.
Offline sources retain cached records and registrations. Availability checks run
on a worker when snapshots refresh, not on every frame.

## Browser checkpoint

The desktop integration provides Libraries and Collections in a narrow
activity rail. The library sidebar shows named banks above their source/folder
tree. Selecting a bank shows all its indexed sounds; selecting a folder scopes
the central table and search to that subtree. `All sounds` restores the bank
scope. Switching to Collections preserves the library, folder, and query.

Create/rename/remove bank and attach/detach source operations run on the storage
worker. Attaching registers the directory before scanning; cancelling the scan
retains the registration and committed batches. Removing a registration leaves
its index rows as rebuildable cache, excluded from registered-bank queries.

The existing index thread owns both the catalog writer and the durable library
connection. The independent read thread serves search and folder pages. Their
command/read/event queues are bounded at 4/4/8. Query and tree generations reject
late answers; accepted edits drain on shutdown even when completion is unread.
The UI holds one 64-row page per open folder, with at most 32 open branches and
visible-row rendering. Collapsing a branch releases its descendant pages.
Failed edits retain their command and name draft for correction, retry, or discard.
Changing sidebar mode leaves the library scope and search text intact. No new
framework, service, runtime, or application thread was introduced.

The CLI `index` command also registers its source as a named bank, reusing an
existing registration. `browse --folder` attaches to the selected bank or
creates `My library` for an empty registry. These developer entry points use
the same source validation and durable storage.

## Limits and evidence

The storage tests cover overlapping roots, stable sample IDs, native Unicode
component boundaries, folder paging, schema-1 migration/rollback, restart,
index rebuild, offline sources, source preservation, and one-time legacy import.
Desktop tests cover failed-draft recovery, form-opening Enter behavior, stale
tree answers, scoped worker responses, and unread-save shutdown. All five
generated WAV/FLAC/MP3/AIFF fixture hashes remain unchanged after indexing.
A native release capture rendered the two-bank browser and four-row selected
bank; it was visually inspected before the final painted-arrow/row polish.
The Windows helper could read controls but could not click an indexed element;
Computer Use was then stopped with physical Escape. Folder/bank mouse selection,
source dialogs, Collections mode switching, and Explorer gestures remain manual
runtime checks. The final build is compiled; no new CPU/RAM qualification is
claimed.

The [incremental live index](INDEXING.md) now skips unchanged metadata and
reconciles moved/deleted cached paths. [Favorites](FAVORITES.md) are implemented;
durable source relinking, multi-library
selection, and restart persistence of the active view are later checkpoints.
Exact locators are deduplicated; distinct hard links or filesystem aliases are
not a content-hash identity system. No performance improvement is claimed from
the data structure alone.

## Clear tree layout and navigation

The approved Clear tree concept is implemented with 30-pixel rows, 16-pixel
indents, fixed chevron/icon/name columns, subtle hierarchy guides, and distinct
hover/selection/keyboard focus. Sources use drive icons, other folders use
closed/open folder icons. Cached source counts sit at the right; offline or
unavailable status is explicit. Truncated labels expose the full native path
on hover. Library rows no longer reserve unused vertical space up to their
180-pixel maximum. The Folders header's labeled **+** opens the existing native
source chooser; source context menus retain rescan/removal actions.

Up/Down move focus through currently loaded rows, Left/Right collapse/expand or
move to the parent/first child, Enter selects the sound scope, and Home/End move
to the first/last loaded row. Space in the tree does not trigger playback.
Focus uses the bank, native source root, and native folder path rather than row
indices; overlapping-root appearances cannot share a widget identity. Selection
survives collapsing its branch. Focus scrolls into view, survives temporary page
invalidation, and falls back to a visible ancestor if its folder disappears.
Tab still leaves controls available; full screen-reader qualification is pending.

Derived rows borrow loaded metadata/native locators. Only visible row widgets
are created, using existing 64-child pages, 32 open branches, generation checks,
and bounded workers. No scanner, source-count query, database schema, dependency,
or worker was added. Stale replies for a previous folder page offset are also
rejected. The first step intentionally does not compute subfolder counts.

The integrated default-feature suite passes 94 tests, including breadcrumb
targets, bounded revealing, separate chevron/row clicks, raw keyboard events,
virtual-row scrolling, focus after refresh/removal, distinct overlapping-root
IDs, and stale offset replies. Formatting, Clippy, and the Windows release build
pass. A real release capture of the approved generated fixture bank shows the
expanded hierarchy, selected Forest folder, full breadcrumb, and its two sounds;
all five WAV/FLAC/MP3/AIFF source hashes remain unchanged. Physical Windows
mouse/keyboard use, context-menu/dialog operation, and screen-reader interaction
remain manual checks. No whole-app performance qualification is claimed here.

One repeated suite run encountered Windows error 5 while renaming a temporary
directory in the existing incremental-index fixture. That test passed in an
isolated serialized run and the normal full-suite repeat passed. The fixture
and index implementation were not changed to mask the failure.

Developer capture can set `SIFT_SMOKE_SUBFOLDER=Ambiences\Forest` together with
`browse --smoke-screenshot PATH.ppm` on an isolated fixture bank. Only normal
relative components are accepted, under the active bank's first native source;
ordinary startup ignores the selector. Raw captures stay in `.local/`.

Design references: [Fluent tree navigation](https://fluent2.microsoft.design/components/web/react/core/tree/usage),
[W3C tree keyboard patterns](https://www.w3.org/WAI/ARIA/apg/patterns/treeview/),
and [breadcrumb guidance](https://www.nngroup.com/articles/breadcrumbs/).
