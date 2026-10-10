# Collections

Collections are named selections of original sound references. A sound can
belong to several collections. Adding it twice to the same collection retains
one reference; removing an item or collection leaves its original file intact.

## Workflow

The Collections sidebar lists names above saved sounds. **Collections +** creates
a selection; context menus rename or delete it. **Add selected sounds** adds the
marks from the last used sound list. Drag a marked group onto a collection name
or its contents heading to add all its references. Selecting the destination
retains marks from the previous collection until you make a new selection there.
Saved items can be previewed and removed through their context menu.

Contents support [multiple marks and keyboard navigation](SELECTION.md).
W/S and Up/Down cross page boundaries; F toggles the active sound's favorite.
Returning to a recently used collection restores its marks, active sound,
content page, and scroll position without starting playback. See
[Navigation](NAVIGATION.md) for session scope and limits.
**Remove selected** removes the marked references from the current collection.
A marked row's context menu removes its marked group; an unmarked row's menu
removes only that reference. Group actions accept 1 to 128 sounds across pages.
Larger groups fail visibly; no prefix is saved or removed. Pending ranges must
finish first. Removing offline references does not require their source files.

Edits save automatically. Conflicting actions are disabled while saving. Failed
edits retain their mutation and name draft for retry or explicit discard. Read
failures remain visible and can be retried. The first collection opens initially;
saved names and membership survive restart independently of the active view.

Internal drags carry captured marks and their source query/scope inside Sift.
Outgoing dragging accepts one sound; a group crossing the client boundary shows
an actionable message without copying a subset. Returning after native handoff
cancels that gesture rather than adding another internal item.

## Durable data

Collections and [Favorites](FAVORITES.md) share `catalog.collections.sqlite`.
Schema 2 uses STRICT tables, foreign keys, WAL, and FULL synchronization.
Schema-1 upgrades are transactional. Unknown versions or unrelated databases
return errors without resetting user data.

Collection and entry IDs are independent, typed, non-reused identities. They
have no foreign-key dependency on rebuildable catalog rows. Entries retain
native source paths, last-known display/audio metadata, revision information,
and stable insertion order. Display strings are not file-operation locators.

Names are trimmed, limited to 120 characters, and unique by a Unicode lowercase
key. Page reads and batch adds are limited to 128 items. Batch changes commit
transactionally. The worker owns both its connection and source inspection;
its command/event queues each hold four messages. Library group resolution uses
one lazy read-only catalog connection on this same worker. Accepted writes drain
during shutdown even after the UI stops reading completion events. Queries,
native marks, and destination IDs are captured when an action is accepted, so
switching views cannot redirect a pending save.

## Source availability

Adding a reference validates the current ordinary file and its metadata bounds.
Missing files, links/reparse points, stale sizes, and malformed references fail
visibly. Existing entries distinguish ready, changed, missing, and inaccessible
sources. Missing references remain saved.

Revision checks use size and available modification timestamps, not content
hashes. Automatic relinking, transcoding, virtual collection folders, item
reordering, outgoing group copying, and portable import/export are not implemented.

## Qualification

Local verification covers restart and index-rebuild preservation, duplicate
addition, non-reused IDs, source states, rollback, names, paging, migration
refusal, unread-save shutdown, draft recovery, and internal drag delivery.
Group checks cover off-page marks, captured source/destination changes, query-wide
exclusions, oversized rejection, offline removal, atomic failures, and shutdown.
Actual Windows captures and keyboard operation have been exercised. Physical
group mouse gestures, context-menu use, Explorer gestures, and accessibility need
broader qualification. Test sources and fixtures are private.
