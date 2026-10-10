# Libraries and folders

A library is a named group of existing source folders. Registering, renaming,
or removing a library changes its registration without moving or deleting audio.
Libraries can span disks and contain overlapping folders. The catalog stores
each exact native file locator once.

## Browsing

The library sidebar lists banks above the folder tree. Selecting a bank shows
all its indexed sounds; selecting a folder scopes the table and search to that
folder and its descendants. **All sounds** restores the bank scope. A bank opens
its last folder and tree position, or all sounds on its first visit. Recently
visited folders remember their own search, marks, and sound-list position.
Switching to Collections preserves the library view. See [Navigation](NAVIGATION.md).

**New** creates a library. **Folders +** opens the native source-folder chooser.
Context menus provide rename/removal and source rescan/removal actions.
Pending edits disable conflicting actions. Failed saves retain the draft for
retry, and accepted registrations survive restart.

The tree contains directories with indexed audio, including their ancestors;
empty directories are omitted. Sources have drive icons and cached counts or
availability labels. Other directories have folder icons. Chevron clicks expand
branches; row clicks select their sounds. Truncated names expose their native
path on hover.

Breadcrumbs select ancestors while preserving the bank. A new folder inherits
the current query; a revisited folder restores its own query. Deep paths
collapse middle labels into a menu. A source named like its bank does not repeat
that label. Revealing ancestors uses loaded metadata without inspecting the
filesystem on the UI thread.

## Keyboard navigation

| Key | Action in the focused tree |
| --- | --- |
| Up / Down | Move through loaded rows |
| Left / Right | Collapse / expand or move to a parent / first child |
| Enter | Select the focused folder's sound scope |
| Home / End | Focus the first / last loaded row |

Focus and sound selection are separate. Collapsing a branch preserves selection.
Focus follows native bank/root/folder identity through refresh and scrolling,
and returns to a visible ancestor when a folder disappears. Space in the tree
does not trigger playback; Tab leaves the controls available.

## Storage and bounds

`catalog.libraries.sqlite` stores schema-1 bank/source registrations with typed,
non-reused IDs, native paths, foreign keys, and transactional writes. Current
limits are 128 banks, 32 sources per bank, and 512 sources overall.
An index rebuild leaves this database and saved-reference data intact.

The schema-3 catalog has a unique native-locator index and derived folder table.
Older-schema migration deduplicates locations transactionally; invalid locators
or unsupported versions fail without resetting data. A one-time legacy-root
import marker prevents removed registrations from returning after restart.

Folder scopes use separator-terminated native-byte ranges so descendants match
without including similarly named siblings. Folder pages have 64 rows, the
desktop keeps at most 32 branches open, and only visible widgets are rendered.
Subfolder counts are not computed in the current browser.

Availability checks run on workers when registrations refresh. Offline sources
retain cached metadata. Incremental scanning and live changes are described in
[Indexing](INDEXING.md). Hard links and filesystem aliases remain distinct
locations; durable source relinking and active-view restoration are later work.

## Qualification

Local verification covers migrations, overlapping roots, native path handling,
subtree bounds, offline registrations, save recovery, breadcrumbs, distinct
chevron/row targets, stale replies, and virtualized keyboard focus. Native
Windows captures retain the generated source bytes. Physical dialogs/context
menus, screen-reader behavior, and representative large libraries still need
broader qualification. Verification sources and captures are maintained locally.
