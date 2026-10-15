# Library metadata filters

Use **Filters** beside the library search field to narrow the sound table.
The same library table remains filtered while Collections is open; Favorites
and saved collection contents keep their own search and sorting.

- **Length:** Any, a range in seconds, or Unknown. Bounds are inclusive; leave
  one bound empty for an open end. Ranges exclude unknown lengths unless
  **Include unknown lengths** is checked. Zero seconds is a valid bound.
- **Format:** select WAV, FLAC, MP3, AIFF, or Ogg. Multiple choices match any
  selected format; no choices means unrestricted. AIFF includes `.aif` and `.aiff`;
  Ogg includes `.ogg` and `.oga` with Vorbis audio.
- **Channels:** Any, Mono, Stereo, or Unknown. Any includes multichannel sounds.
- **Sample rate:** Any, a listed rate in kHz, or Unknown.

Valid changes update results immediately; there is no Apply step. The length,
channel and sample-rate controls open nested choice menus. Selecting a choice
closes that menu while retaining the filter panel. Invalid numbers,
negative/nonfinite bounds and reversed ranges display an error and retain the
last valid results until corrected. Emptying both length bounds clears that
criterion. Close, Escape or clicking outside dismisses the panel and retains
applied filters; reopening restores the valid settings. Active filter buttons
appear below search; click one to remove that criterion or use **Clear filters**
to remove all metadata filters while retaining the text query.

Criteria combine with text and the selected library/folder. For example, search
`impact` and choose Stereo with a maximum length of 3 seconds to find short
stereo impacts. Filtering reads existing indexed metadata, not the audio files.
Unknown means a missing value; an analysis error remains visible in Details.

Applying or clearing filters resets the sound marks, range anchor and list
position, as changing the text query does. The current preview continues;
navigation activates a sound from the new result list. Ctrl+A, Shift ranges,
sorting, collection adds and outgoing copies all use the applied criteria,
including unloaded pages. An accepted group action keeps its captured criteria
when the view changes. The existing 128-sound action limit still applies.

Filters are remembered per library/folder during the open session, together with
query and sorting. An unvisited folder inherits the current library's settings;
a different library starts unfiltered. Revisiting a folder restores its own
settings. Unapplied popup drafts are transient. This feature does not persist
filters across restart or change saved membership, source audio or schemas.

The catalog owns validated predicates shared by pages, counts, ranges, native
identity lookup and group resolution. Filter changes invalidate result pages
and stale replies; indexed metadata changes invalidate counts and paging anchors.
Returned pages stay bounded to 128 rows. Broad combined predicates may still
scan/sort many candidates; see [Performance](PERFORMANCE.md) for measurement limits.

## SQL measurement

A release measurement on 2026-10-12 used the fixed 100,000-row metadata-only
fixture and host described in [Search measurements](SEARCH-PERFORMANCE.md).
It tested filename and length ordering in both directions, with two warmups and
20 timed repetitions after first use. A deep page starts 200 rows before the end;
page walks request 20 adjacent 128-row pages near the end. Independent fixture
arithmetic checks membership and ordered IDs, outside the timed intervals.

| Applied criteria | Matches | First-page warm p95 | Deep-page warm p95 | Page-walk p95 |
| --- | --- | --- | --- | --- |
| Length at most 3 s, stereo | 4,329 | 0.393–39.215 ms | 0.513–44.643 ms | 1.113–40.136 ms |
| WAV, 48 kHz | 50,000 | 0.407–35.761 ms | 0.410–40.861 ms | 0.428–1.345 ms |
| Text `sound`, length at most 3 s, stereo | 4,329 | 41.986–45.889 ms | 38.707–42.909 ms | 7.723–41.126 ms |
| Folder A, length at most 3 s, mono | 4,329 | 0.622–58.141 ms | 0.684–53.064 ms | 1.120–49.867 ms |
| Unknown length | 14,286 | 0.235–0.480 ms | 0.304–0.517 ms | 0.579–0.924 ms |

Ranges span the four sort/direction combinations. First use of each connection
and criteria, including counting, took 1.644–110.673 ms. Deep-page first use
already benefits from that connection's verified count. OS/SQLite caches were
not flushed; desktop activity and processor scheduling were not controlled.
These are component timings, without debounce, rendering, indexing or playback.
They do not establish end-to-end search latency on a representative audio library.

The existing unfiltered harness also retained all 240 totals, row counts and
ordered-ID signatures. Its contemporaneous older/newer build repeats and timing
variation are recorded in [Search measurements](SEARCH-PERFORMANCE.md#metadata-filter-repeat).
No dependency, schema migration, audio scan or full result cache was introduced.
