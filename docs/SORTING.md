# Sound ordering

The storage APIs support ascending and descending ordering by filename, format,
duration, sample rate, and channel count. Collection entries also support their
original insertion order. Desktop controls are a separate integration step;
the current desktop still uses filename order and collection insertion order.

Filename keys use Rust's Unicode lowercase conversion. Numeric metadata sorts
numerically, with unknown values last in both directions. Equal primary values
use filename key and row ID in the selected direction. This provides a stable
total order across bounded pages, including similarly named sounds. Reversing
the full key lets the same SQLite index serve both directions.

Page reads, inclusive Shift ranges, and native-identity cursor/anchor lookup
share that ordering rule. Endpoints and results are resolved in one SQL snapshot;
missing endpoints remain explicit. Position lookup counts preceding matches
without loading their metadata into application memory.

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
