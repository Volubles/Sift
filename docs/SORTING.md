# Sound ordering

Click a column heading in Libraries or Favorites to sort by **Filename**,
**Format**, **Length**, **Rate**, or **Channels**. Clicking the same heading
reverses direction; an arrow marks the active heading. A different heading
starts in ascending order.

Collection contents use a compact **Sort** menu with the same fields and an
**Added order** option. The arrow button reverses direction. New collections
start in their original insertion order; sorting changes the view and does not
rewrite saved membership or insertion order.

Completed marks keep their native source identities. The active preview
continues without restarting. A background lookup relocates the active row and
range anchor, including off-page sounds, and reveals the active row in the new
order. W/S, arrow navigation, and Shift ranges use the new positions once
lookup finishes. Navigation and new ranges wait while lookup is pending;
lookup failures retain marks and provide an explicit retry.

Order is remembered per library/folder and collection during the open session.
An unvisited folder inherits the order within its library; a different library
starts with filename order. Favorites retains its own order. Query changes
retain ordering; [navigation](NAVIGATION.md) describes the other saved view state.

Filename keys use Rust's Unicode lowercase conversion. Numeric metadata sorts
numerically, with unknown values last in both directions. Equal primary values
use filename key and row ID in the selected direction. This provides a stable
total order across bounded pages, including similarly named sounds. Reversing
the full key lets the same SQLite index serve both directions.

Page reads, inclusive Shift ranges, and native-identity cursor/anchor lookup
share that ordering rule. Endpoints and results are resolved in one SQL snapshot;
missing endpoints remain explicit. Position lookup counts preceding matches
without loading their metadata into application memory.

Collection adds and outgoing group drags capture the source order together with
query, scope, and marks. Later view changes do not redirect an accepted action.

Catalog schema 4 adds indexes for the four metadata columns. Saved-reference
schema 3 adds Unicode filename keys to collection entries and indexes for
collection/favorite ordering. Existing entry IDs, membership, favorites, native
paths, and original audio remain intact. Collection filename keys are populated
in bounded batches; each migration and its index creation are transactional.
Readers require the current schema and never create or migrate storage.

Index construction is one-time work on the storage's owning worker, and the
additional indexes consume disk space and participate in metadata writes.
Scoped or text-filtered queries may require SQLite sorting depending on the
query plan. Bounded returned pages do not by themselves establish latency or
whole-application performance on a representative library.

## SQL measurement

A Windows 10 release run with SQLite 3.53.2 on an i9-13900K (32 logical CPUs)
used 100,000 generated metadata rows, ten field/direction combinations, three
warmups, and twenty repetitions per scenario. Each page returned 128 rows.
Across the ten orders, warm p95 page latency was:

| Scenario | p95 range |
| --- | --- |
| All sounds, first page | 11.8–13.8 ms |
| 50,000-row folder, offset 49,800 | 82.5–185.8 ms |
| Text query matching all 100,000 rows, first page | 77.4–179.2 ms |

Cursor lookup p95 was 0.8–8.9 ms for all sounds, 29.2–32.7 ms in the folder,
and 58.5–62.3 ms for the broad text query. These are SQL component measurements
on a local temporary database, with no audio files, rendering, or device work.
Other desktop activity was not recorded. They do not certify the end-to-end
search target, memory use, or representative-library performance. Broad-match
queries and deep folder pages remain concrete optimization work.

The [subsequent search/pagination comparison](SEARCH-PERFORMANCE.md) uses one
fixed fixture for both revisions and includes sequential and random deep pages.
It supersedes these initial costs for the optimized catalog; neither run
qualifies whole-application search or audio latency.
