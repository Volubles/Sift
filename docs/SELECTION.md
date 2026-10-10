# Sound selection and keyboard navigation

Libraries, Favorites, and collection contents support marking several sounds.
The active row has a thin outline; activating a sound updates preview. Other marked sounds keep
their selection background. Keyboard commands apply to the focused sound list,
leaving search fields, forms, folder-tree navigation, and menus available.

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

## Pages, updates, and scope

Each list retains one active reference for shortcuts even when its page is evicted.
Navigation requests the required page and activates the latest requested row
when it arrives. Repeated movement can coalesce while a page loads. Collection
navigation crosses the existing 128-item page boundary automatically.

Marks follow exact native source paths rather than display names or catalog
IDs. Completed marks survive page eviction and refresh; changing the search,
library/folder scope, or active collection clears them. A pending range is
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

Adding to a collection, removing an entry, and outgoing dragging currently act
on one active/dragged sound. Group collection edits and copy dragging are a
separate integration step; marking several rows does not imply group export.
Source audio and durable saved data are not altered by marking or navigation.

Local verification covers native-locator range resolution, scope boundaries,
index rebuilds, stale replies, real text-field focus, keyboard repeat, unloaded
rows, collection page transitions, and committed favorite changes. Physical
keyboard audition and audible behavior retain their Windows qualification
requirements. Verification sources and fixtures remain private.
