# Performance

Sift uses bounded result pages, worker queues, and derived caches. Visible rows
are rendered on demand; idle workers wait for commands or native signals.
These resource controls describe the implementation, not a guarantee for every
library or device.

## Design targets

The following provisional targets use a Windows release build, 100,000 indexed
metadata records, a local SSD, and a loaded library without scanning/playback.
They are design targets, not achieved results or service guarantees.

| Metric | Proposed target | Definition |
| --- | --- | --- |
| Idle CPU | At most 0.5% of total machine CPU | Mean over 60 s after stabilization, all Sift processes; record logical cores. |
| Idle memory | At most 150 MiB private process bytes | Same scenario; report working set, GPU memory, and mapped indexes separately. |
| Warm startup | Usable library within 1 s | Process start to interactive list; existing index, no reanalysis. |
| Text search | p95 at most 100 ms | Input to correct visible results, including debounce and rendering. |
| Warm preview | p95 at most 150 ms | Play/selection to first audible sound; state cache, device, buffer size. |
| Scrolling | p95 CPU frame work within 16.7 ms at 60 Hz | Measure active scrolling and visible rows; also report dropped frames. |

Usable startup means an interactive list, and preview latency means audible
output. Window creation, a queued CPAL block, and a worker-ready signal measure
different stages and cannot substitute for those end-to-end observations.

## Recorded measurements

- [Renderer and audio resource observations](BASELINE.md) compare Glow/WGPU and
  document idle/activity limits for the desktop.
- [The 100,000-file index measurement](INDEXING-BASELINE.md) records initial
  import, unchanged inventories, native update latency, and headless resources.
- [Sound ordering](SORTING.md#sql-measurement) records warm ordered SQL pages and
  cursor lookup on 100,000 generated metadata rows, including broad-match and
  deep-folder limitations.
- [Search and pagination comparison](SEARCH-PERFORMANCE.md) records before/after
  latency, correctness signatures, CPU time, and process memory on one fixed
  100,000-row fixture.

The index fixture used tiny generated WAV files. Unchanged refreshes improved,
while initial import remained slower than the preceding implementation.
Neither that component result nor the metadata-only desktop catalog qualifies
whole-application performance on representative sound libraries.

## Measurement coverage

Comparisons use the same build profile, hardware, power settings, and defined
cache state. Reports identify file count/bytes, format and duration distribution,
index size, OS/storage/device configuration, operation boundaries, repetitions,
latency percentiles, process CPU normalization, memory, and observed activity.
GPU costs and audible latency are separate measurements.

Current qualification leaves representative long files, external drives/NAS,
device loss, rapid seeking during indexing, active scrolling, large collections,
repeated startup/shutdown, and sustained analysis as further scenarios. Synthetic
metadata is suitable for list-loading checks; real generated audio is needed for
decoding and inventory measurements.

Raw reports, fixtures, and measurement helpers are maintained locally. Published
summaries retain their methods, results, and limitations. No new performance
claim follows from source layout or documentation changes alone.
