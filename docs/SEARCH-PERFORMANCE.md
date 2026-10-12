# Search and pagination measurements

Measured on 2026-10-12 using one fixed 100,000-row catalog before and after
the search changes. Warm page latency improved most for deep folder and broad
text results. Totals, returned row counts, and ordered-ID signatures match in
all 240 recorded metric rows. The database SHA-256 stayed unchanged.

These measurements cover the catalog API, including count validation and
metadata loading. They exclude UI debounce, rendering, audio, and indexing.

## Conditions

- Windows 10 Pro 19045; Intel Core i9-13900K, 32 logical processors; Kingston
  KC3000 NVMe storage; High performance power plan, unchanged between runs.
- Pinned Rust 1.98.1 release profile and bundled SQLite 3.53.2; catalog schema 4.
  Baseline source: `c8df7a7`, before the search/pagination changes.
- 100,000 generated metadata records in a 79.13 MiB database. No audio files
  are loaded or scanned. Two native folder scopes each contain 50,000 records.
- Names comprise 99,900 `sound` matches and 100 `needle` matches. Formats are
  WAV/FLAC; durations span 0–30 seconds with every seventh value unknown;
  sample rates are 44.1/48 kHz and channel counts are one/two. Records advertise
  44 bytes each; those sizes are metadata, not a real audio corpus.
- Five sort columns in both directions, nine scenarios: 90 combinations.
  Each opens a fresh reader, records one first-use call, warms twice, then times
  twenty repeated pages and native position lookups. OS file caches were not
  flushed, and unrelated desktop activity was not recorded.
- Larger views also load twenty different consecutive 128-row pages, starting
  20,000 rows before the end. These exercise page transitions rather than
  repeatedly reading the same page. Each query remains unchanged within a run.
- p50 averages the middle two samples; p95 uses the nineteenth of twenty
  sorted samples. Ranges below span the ten column/direction combinations.
  They are observed single-run component results, not a confidence interval.

## Warm page latency

| Scenario | Matches / offset | Before p95 | After p95 |
| --- | --- | --- | --- |
| All sounds, first page | 100,000 / 0 | 13.058–14.236 ms | 0.200–0.558 ms |
| All sounds, deep page | 100,000 / 99,800 | 16.132–18.342 ms | 0.204–0.398 ms |
| Folder, first page | 50,000 / 0 | 39.061–81.349 ms | 32.063–48.001 ms |
| Folder, deep page | 50,000 / 49,800 | 180.252–313.369 ms | 32.010–46.490 ms |
| Broad text, first page | 99,900 / 0 | 73.398–156.248 ms | 32.259–60.440 ms |
| Broad text, deep page | 99,900 / 99,700 | 258.065–602.482 ms | 32.542–61.426 ms |
| Selective text | 100 / 0 | 0.269–0.458 ms | 0.241–0.386 ms |
| Two-character prefix | 100 / 0 | 0.131–0.217 ms | 0.177–0.385 ms |
| No matches | 0 / 0 | 0.049–0.115 ms | 0.006–0.031 ms |

The prefix path incurs small additional snapshot/page bookkeeping and was
slower in this run. Its p50 range changed from 0.127–0.196 to 0.171–0.196 ms;
the higher p95 is retained rather than treating every query as improved.

First-use calls still compute a count. Across first/deep scenarios, after-change
first-use observations were 12.621–14.907 ms for all sounds, 38.748–56.327 ms
for folders, and 58.642–86.800 ms for broad text. These are single observations
on fresh readers with uncontrolled OS cache state, not cold-start percentiles.
An index commit or changed criteria requires counting again.

## Consecutive pages

The following combines both starting scenarios per view. Returned ordered-ID
signatures matched before/after across each twenty-page sequence.

| View | Before page-walk p95 | After page-walk p95 |
| --- | --- | --- |
| All sounds | 15.045–18.791 ms | 0.246–1.037 ms |
| Folder | 169.877–267.406 ms | 32.061–39.423 ms |
| Broad text | 230.021–565.830 ms | 31.583–40.153 ms |

Native position lookups were also checked. Warm p95 for a present identity
was 0.054–6.695 to 0.015–5.735 ms in all sounds and 29.541–41.649 to
30.215–34.028 ms in folders. Folder relocation remains substantial work.
The known identity does not match the broad `sound` query; those timings only
measure rejecting an absent identity and do not qualify broad-match relocation.

## Process cost

The same complete release harness ran sequentially, with no UI or audio device.
Win32 process counters were sampled every 50 ms. CPU time sums user and kernel
time; peak commit and working set are native process peaks.

| Metric | Before | After |
| --- | --- | --- |
| Harness wall time | 429.828 s | 80.172 s |
| Process CPU time | 410.953 s | 79.641 s |
| Sampled peak private bytes | 11.137 MiB | 7.078 MiB |
| Peak commit | 11.145 MiB | 7.078 MiB |
| Peak working set | 14.066 MiB | 11.355 MiB |

CPU time fell by 80.6% for this workload. Mean CPU use normalized over wall
time and all 32 logical processors was 2.99% before and 3.10% after: faster
completion reduces total work without implying lower instantaneous utilization.
Sampling can miss short private-memory peaks. These values describe the test
process, not Sift's desktop idle memory or whole-application resource budget.

## Implementation and limits

The catalog owns one count memo with a 64 KiB descriptor limit, identified by
SQL source, predicates, and exact native bindings. A read snapshot precedes
same-connection data-version and own-change checks, so a reused count and its
page observe the same database state. External commits, local writes, and
criteria changes invalidate it. SQLite documents the connection-specific
semantics of [data_version](https://www.sqlite.org/pragma.html#pragma_data_version).

Text queries join FTS IDs to live samples. Page queries order compact IDs first
and load wide metadata only for the chosen 128 rows. A component profile of
the old Length-ascending deep-folder query measured 206.494 ms p95 for sorting
wide rows versus 58.356 ms for deferred metadata; the old unrestricted count
alone took 13.192 ms. This supported changing repeated counts and wide sorting
before adding another index or a full result cache.

One bounded page anchor retains actual first/last SQL keys, at most 2 KiB of
key data per endpoint. Matching criteria, revision, order, and adjacent offset
permit seeking before/after that key. Unknown values and tie keys retain the
same visible order in both directions. Unrestricted nullable metadata orders
keep indexed offset traversal because seeking through known/NULL groups was
slower in the intermediate measurement.

Other offsets can start from the nearer end using the verified count. Reverse
traversal reverses NULL placement and tie keys, then restores visible ordering
on the bounded page, including a short final page. This helps near-end jumps;
arbitrary middle pages and broad FTS matches can still scan/sort many candidates.
SQLite's temporary sorting storage can exceed the size of the returned page;
see [temporary files](https://www.sqlite.org/tempfiles.html).

No full result-ID table, new worker, schema migration, or dependency was added.
Durable saved references and original audio are unchanged. Local regression
cases cover concurrent commits, invalidation, native binding/key limits,
orphaned FTS documents, both traversal directions, duplicate keys, unknown
metadata, arbitrary offsets, empty results, and the final partial page.

Raw CSV, process samples, archived benchmark binaries, fixture hashes, and
helpers remain local. End-to-end search during indexing, representative sound
names, long audio, external drives, active scrolling, and GPU/device costs need
their own measurements; this comparison does not certify the desktop search
budget in [Performance](PERFORMANCE.md).

## View-ownership cleanup repeat

A release repeat on 2026-10-12 after the view-ownership cleanup used the same
fixture, harness, power plan and protocol above. All 240 totals, row counts and
ordered-ID signatures match the preceding optimized build; the database hash
is unchanged.

| Warm page scenario | Before cleanup p95 | After cleanup p95 |
| --- | --- | --- |
| All sounds, first page | 0.200–0.558 ms | 0.203–0.598 ms |
| Folder, deep page | 32.010–46.490 ms | 31.743–46.177 ms |
| Broad text, deep page | 32.542–61.426 ms | 31.159–63.220 ms |

Harness wall time was 80.172 to 79.813 seconds; process CPU time was 79.641 to
79.078 seconds. Sampled peak private memory was 7.078 to 6.750 MiB, and peak
working set was 11.355 to 11.359 MiB. Total work and the larger page timings
remain similar in these single runs. Smaller operations also varied: empty-query
p95 spanned 0.006–0.031 before and 0.006–0.138 ms after; this outlier is retained.
These observations support preserving the measured catalog behavior through
the cleanup and do not qualify desktop frame cost or end-to-end search latency.
