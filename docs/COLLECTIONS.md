# Collections

Collections are named selections of original sound references. A sound can
belong to several collections. Adding it twice to the same collection retains
one reference; removing an item or collection leaves its original file intact.

## Workflow

The Collections sidebar lists names above saved sounds. **Collections +** creates
a selection; context menus rename or delete it. **Add selected sound** or a drag
onto a collection name adds the selected reference. Saved items can be previewed
and removed through their context menu.

Edits save automatically. Conflicting actions are disabled while saving. Failed
edits retain their mutation and name draft for retry or explicit discard. Read
failures remain visible and can be retried. The first collection opens initially;
saved names and membership survive restart independently of the active view.

Internal drags carry references inside Sift. Crossing the client boundary hands
the reference to the native copy-only adapter. Returning after native handoff
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
its command/event queues each hold four messages. Accepted writes drain during
shutdown even after the UI stops reading completion events.

## Source availability

Adding a reference validates the current ordinary file and its metadata bounds.
Missing files, links/reparse points, stale sizes, and malformed references fail
visibly. Existing entries distinguish ready, changed, missing, and inaccessible
sources. Missing references remain saved.

Revision checks use size and available modification timestamps, not content
hashes. Automatic relinking, transcoding, virtual collection folders, item
reordering, multi-selection, and portable import/export are not implemented.

## Qualification

Local verification covers restart and index-rebuild preservation, duplicate
addition, non-reused IDs, source states, rollback, names, paging, migration
refusal, unread-save shutdown, draft recovery, and internal drag delivery.
Actual Windows captures and keyboard operation have been exercised. Physical
mouse/context-menu use, Explorer gestures, and accessibility need broader
qualification. Test sources and fixtures are private.
