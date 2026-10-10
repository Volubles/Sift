# Favorites

Favorites are global saved sound references. They are independent of library
registration, collection membership, and rebuildable catalog IDs. Unfavoriting
a sound never removes it from a collection or changes its original file.

## Durable foundation

Favorites share `catalog.collections.sqlite` and its existing owner thread with
Collections. Schema 2 adds a STRICT favorites table, non-reused `FavoriteId`, a
unique lossless native locator, saved display/audio metadata, and source revision.
Schema 1 upgrades transactionally without changing collection/entry IDs. Failed
migrations and unsupported versions preserve existing user data.

Search uses the same literal trigram FTS5 and indexed short filename prefixes as
Libraries. Results sort by lowercase filename then ID, return at most 128 rows,
and count/rows share a database snapshot. Only returned rows check filesystem
availability. Missing, changed, and inaccessible references stay visible.

Saving validates the reference and current ordinary file; stale sizes fail
visibly. Removal requires only its saved native path, so it works offline.
Repeated explicit saves/removals are idempotent. File moves do not automatically
relink references; exact native locators remain the identity boundary in this
checkpoint. Alias/content identity and safe relinking are separate work.

Membership reads perform indexed native-path lookups for at most 641 paths and
1 MiB encoded path data. There is no whole-favorites in-memory set. The existing
four-message command/event queues gain separate favorite-page and membership
generations; stale reads cannot replace newer requests. Accepted edits drain on
shutdown even after completion events stop being read. No extra thread, runtime,
connection, or dependency is introduced.

## Foundation verification

Generated WAV/disposable database tests cover restart and index rebuild,
collection deletion independence, duplicate saves, non-reused IDs, literal
search, short prefixes, paging, source states, offline removal, failed saves,
request bounds, migration preservation/rollback, and unread-save shutdown.
The foundation passes 58 headless tests, formatting, and headless Clippy.
Desktop stars and the global Favorites view are the next integration checkpoint.
