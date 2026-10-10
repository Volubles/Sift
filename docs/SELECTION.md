# Sound selection and keyboard navigation

Libraries, Favorites, and collection contents support marking several sounds.
The active row has a thin outline; activating a sound updates preview. Other marked sounds keep
their selection background. Keyboard commands apply to the focused sound list,
leaving search fields, forms, folder-tree navigation, and menus available.

On startup, the loaded library sound list receives keyboard focus once if no
other control owns it. S or Down can navigate immediately without clicking a
sound first. A search field or open menu keeps its focus; switching views also
ends this initial focus request. Loading or empty results do not consume it.

| Input | Action |
| --- | --- |
| Click | Mark and activate one sound |
| Ctrl+click | Toggle that sound's mark and activate it |
| Shift+click | Mark the range from the anchor to that sound |
| Ctrl+Shift+click | Add a range to existing marks |
| Ctrl+A | Mark every result in the current view, including unloaded pages |
| W / Up | Activate the previous sound |
| S / Down | Activate the next sound |
| Shift+W/S or Shift+Up/Down | Extend the marked range while navigating |
| F | Toggle the active sound's global favorite |
| Space | Play/pause the preview |

Auto-play starts enabled. Clicking or navigating to a sound auditions it when
auto-play is on; turning it off retains navigation and waveform preparation.
The initial passive catalog selection prepares a waveform without starting
audio. Marking all results or starring a sound does not restart playback.
Holding F does not repeatedly toggle the star. Pending favorite saves retain
the existing edit guard; failures remain available for retry or discard.

Keyboard audition keeps the current scroll position while the activated row is
fully visible. Crossing the viewport edge scrolls only enough to reveal that row;
the list does not recenter it. This also applies to Shift-navigation and uses the
collection content viewport when the sidebar list has a limited height.

## Pages, updates, and scope

Each list retains one active reference for shortcuts even when its page is evicted.
Navigation requests the required page and activates the latest requested row
when it arrives. Repeated movement can coalesce while a page loads. Collection
navigation crosses the existing 128-item page boundary automatically.

Marks follow exact native source paths rather than display names or catalog
IDs. Completed marks survive page eviction and refresh. Changing the search
clears that view's marks; [session navigation](NAVIGATION.md) restores completed
marks when returning to a recently used library/folder or collection. A pending range is
cancelled if its result context changes, and late replies cannot replace newer
marks. Missing endpoints produce feedback without guessing a replacement.

Ctrl+A represents the current view as a whole, with individually excluded paths.
It does not eagerly read all metadata or audio; new results in the same view
are included after refresh. Explicit ranges resolve on the existing SQL worker
using one snapshot and the current view's order: filename/ID for library and
favorite results, insertion order for collection entries.

Range replies and explicit mark/exclusion sets have limits of 1,000,000 paths
and 8 MiB of path data each. Oversized ranges leave previous marks intact and
suggest Ctrl+A. No operation silently truncates a selection.

## Current action scope

F acts on the active sound. If membership is still unknown, the storage worker
toggles the committed state transactionally and returns the resulting state;
an older status reply cannot undo it. Existing favorites can still be removed
when their source is offline.

**Add selected sounds** uses the marks from the last used sound list. Internal
drags from Libraries, Favorites, or a collection carry the entire marked group.
Dragging an unmarked row replaces the marks with that row. Selecting a collection
destination retains prior collection marks until you make a new selection there.
**Remove selected** removes the current collection's marked references. A marked
row's context menu removes its marked group; an unmarked row removes only itself.

Each group action accepts 1 to 128 sounds, including marked sounds outside loaded
pages. Ctrl+A resolves the captured query and scope on the storage worker; explicit
marks resolve exact native paths in that view. Missing explicit members, oversized
groups, and pending ranges fail visibly without committing a partial operation.
Accepted commands keep their source view and destination even if you switch views.
Failed saves retain the command for retry or discard. Duplicate additions keep
one reference per source in a collection; removal also works for offline sources.

Outgoing dragging offers the complete marked group of up to 128 original files.
Selection lookup and file preparation run in the background using the captured
source view. Releasing or pressing Esc during preparation cancels the gesture;
late results cannot start a new drag. The details control offers its one sound.
Source audio and durable saved data are not altered by marking or navigation.

Local verification covers native-locator range resolution, scope boundaries,
index rebuilds, stale replies, real text-field focus, keyboard repeat, unloaded
rows, collection page transitions, and committed favorite changes. Physical
keyboard audition and audible behavior retain their Windows qualification
requirements. Verification sources and fixtures remain private.
