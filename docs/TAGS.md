# Manual tags

Select a sound or mark a group to edit its tags below the waveform. Type into
**Add tag...** and press Enter or **Add**. Matching names are reused; **Existing**
offers up to 24 names matching the current prefix. Changes save automatically.
Failures retain their captured target and show **Retry tag edit**, even after
moving to another sound.

The panel shows the union of selected sounds' tags. `Metal (1/3)` means that one
of the three sounds has it. Adding applies only that association; the adjacent
**×** removes only that tag from the selected sources. Their other tags remain.

Chips size to their text, with a fixed height and rounded corners. They wrap in
a scrollable editor capped at 90 logical pixels; large summaries page through
32 chips. Wide Libraries and Favorites tables include a single-line Tags column.
It displays up to two chips and an overflow count, preserving the 28-pixel row.
Narrow tables omit the column; the editor remains available. Collection sounds
share the same global associations through the editor.

Use **Tags** beside library search to select criteria. All chosen tags must match,
together with text, folder scope and metadata filters. Click an active chip to
remove it, or **Clear tag filters** to remove only tag criteria. Clicking a chip
in Libraries filters the current library/folder. Clicking from Favorites or a
collection opens matching sounds across all libraries and clears text/metadata
criteria. Applied tag criteria are remembered per library/folder during the
session. Changing criteria resets marks and scrolling, as ordinary search does.

Tags follow the native source locator. Renaming/moving a source does not transfer
its associations automatically, and identical bytes at another locator do not
share tags. Associations at the old locator remain preserved for explicit recovery.
Global tag renaming/deletion, colors and relinking are further features.

The storage API supports transactional definition rename, merge and deletion.
Renaming preserves identity and rejects another tag's name. Merging keeps the
destination identity, unions associations without duplicates, and removes the
source definition. Deletion removes only that definition and its associations.
Missing sources participate without file access. These operations need no schema
migration. Definition browsing uses name-prefix keyset pages of at most 32 tags
with global association counts, including empty definitions and missing sources.
The background tag owner serializes accepted definition changes with association
edits and drains them on shutdown; read replies carry independent generations.

The tag store holds global labels and their native source associations in
`catalog.tags.sqlite`. Tag identities are independent of rebuildable catalog
rows, collection membership and favorites. Creating or removing an association
does not inspect, rename or write the source audio. Missing sources retain tags;
a replacement locator is not inferred automatically.

Names contain 1–40 characters, are trimmed and compared using a lowercase key.
Reusing a name adds its existing tag. Each sound supports 32 tags; edits accept
up to 128 sounds and a 1 MiB combined native-locator budget. Batch additions and
removals are transactions. A failed addition rolls back every association and
any newly created definition, preserving unrelated tags. Empty definitions remain
available for reuse after removing their last association.

The version-1 database rejects unsupported versions and foreign populated files
without resetting their contents. One background owner has four-slot command and
event queues. Reads have independent generations; accepted edits drain at shutdown.
Visible membership reads are bounded to 641 sources and 32 tags per source;
suggestions return at most 24 names. Selection summaries resolve the same captured
library, favorite or collection intent as other group actions.

Library queries carry up to eight unique tag identities. All selected tags must
match, together with text, scope and metadata filters. Pages, marks, ranges,
identity lookup and outgoing copies share those predicates. A lazy read-only
attachment keeps tag data separate from the index. Non-text counts join from
the first tag's candidates; ordered pages and ranges use indexed membership
checks without sorting the full tag set. Text queries retain the FTS/tag join.
Count and page snapshots observe both databases;
commits to either invalidate retained counts and anchors. No audio data or full
result-ID list is loaded to filter tags. Ordinary unfiltered search does not
require attaching tag storage.

The UI loads source membership in bounded batches on the tag worker. It does not
open storage or inspect files during drawing. Independent generations reject late
membership, summary and suggestion replies. A tag edit affecting membership of
an active library filter refreshes that result list and clears its marks.

## SQL measurement

A release comparison on 2026-10-12 used the fixed 100,000-row metadata fixture
and host in [Search measurements](SEARCH-PERFORMANCE.md), with 126,667 tag links.
The same query protocol checked filename/length ordering in both directions,
first pages and pages 200 matches before the end. Two warmups and 20 repetitions
followed first use. Independent fixture arithmetic checked every ordered ID;
all 32 totals, row counts and signatures match before and after query tuning.

| Criteria | Matches | Before warm page p95 | After warm page p95 |
| --- | --- | --- | --- |
| One tag | 10,000 | 54.518–61.637 ms | 0.696–9.635 ms |
| Two tags and text `sound` | 3,300 | 138.896–167.307 ms | 114.055–135.114 ms |
| Tag on every sound | 100,000 | 165.059–197.323 ms | 0.461–0.704 ms |
| Two tags, length at most 3 s | 288 | 53.158–56.410 ms | 15.530–44.407 ms |

The initial tag join sorted all candidates for every non-text page. A pinned
SQLite profile measured one-tag filename pages at 50.454 ms p95 using that join,
versus 1.717 ms using indexed ordered samples with membership checks. Counts
favored the tag-candidate join, so the query owner now compiles separate count
and page access paths from the same criteria and bindings. Text retains its
measured FTS/tag plan; forcing tag-first FTS traversal was slower in profiling.

New-connection first-page calls, including counting, took 21.869–232.738 ms after
tuning. Broad text plus tags remains above the provisional search budget even
before debounce/rendering; it is a remaining optimization scenario. OS caches
were not flushed, and desktop activity/scheduling were uncontrolled. These are
SQL component timings without audio, indexing or UI, not whole-application
qualification. The original catalog hash is unchanged; raw reports, archived
binaries, tag fixture and profile alternatives remain private.
