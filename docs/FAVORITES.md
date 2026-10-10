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
The foundation passed 58 headless tests, formatting, and headless Clippy.

## Desktop workflow

Click a sound's star in Libraries or Collections to save/remove its favorite.
The third rail icon opens the global Favorites table. Its search is independent
of the preserved Library query/folder/bank, including when a favorite's bank has
been detached. Selection uses `FavoriteId`, separate from catalog/collection IDs.
Preview and internal reference dragging reuse the existing source-validation,
waveform/audio, and external copy-only routes. Switch to Collections to add the
selected favorite there; unfavoriting leaves that collection unchanged.

Only visible rows render. The Favorites view retains four 128-row pages. Star
membership tracks the four cached Library pages, the loaded Collection page,
and the selected source. Cached status is rebuilt only when those paths change
or an edit commits; idle/playback frames do not recopy that path set. Separate
query/view generations discard stale data while preserving scroll during refresh.
Changes in watched source paths refresh affected cached saved-source pages.
Unwatched/offline sources can be checked with **Refresh favorites**.

Stars change only after a successful commit. Unknown membership disables the
star while it loads. Pending saves disable another favorite edit; failure keeps
the mutation and last confirmed star for explicit retry/discard. Feedback is
visible in every view. A superseded worker query still wakes the UI after freeing
queue capacity, so a request rejected by backpressure can retry without polling.

The integrated suite passes 86 default-feature tests and 59 headless tests;
formatting, Clippy, and the Windows release build pass. Controller tests cover
shared membership, offline removal without collection deletion, failed saves,
late membership/search replies, page limits, query validation, queue recovery,
and star clicks that do not select/drag the adjacent row.

Actual release Windows captures of Libraries, Collections, and Favorites show filled/empty
stars, the rail, shared saved sounds, and a retained missing source. Five
generated WAV/FLAC/MP3/AIFF source SHA-256 hashes remain unchanged. Captures are
rendering evidence; physical favorite clicks, listening, and Favorite-to-Explorer
mouse drops remain manual checks. No new whole-app CPU/RAM or large-favorites
latency qualification is claimed from this checkpoint.

Developer capture: use an isolated seeded fixture database with
`browse --smoke-screenshot PATH.ppm`. `SIFT_SMOKE_VIEW=favorites` or `collections`
selects that view only when a smoke capture was explicitly requested. Ordinary
startup ignores this developer selector. Raw captures/fixtures stay in `.local/`.
