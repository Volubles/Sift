# Performance and evidence

## Status and proposed budgets

No release performance baseline exists yet. A native debug smoke test has
rendered a 100,000-record synthetic catalog while loading 128 metadata records
into the view; this is bounded-loading evidence, not CPU/RAM or latency proof.
See [checkpoint verification](CHECKPOINT.md). The following are proposed prototype
budgets, not user-confirmed SLAs or achieved results. They make efficiency
measurable. Revise them through a reasoned product decision after measurement;
do not silently raise them to make a regression pass.

Reference scenario: Windows, local SSD, release build, 100,000 indexed metadata
records, a bounded result page, no scanning or playback. Record actual hardware
and content. Synthetic metadata is not a substitute for real files in audio
or scanning measurements.

| Metric | Proposed target | Definition |
| --- | --- | --- |
| Idle CPU | At most 0.5% of total machine CPU | Mean over 60 s after stabilization, all Sift processes; record logical cores. |
| Idle memory | At most 150 MiB private process bytes | Same scenario; report working set, GPU memory, and mapped indexes separately. |
| Warm startup | Usable library within 1 s | Process start to interactive list; existing index, no reanalysis. |
| Text search | p95 at most 100 ms | Input to correct visible results, including debounce and rendering. |
| Warm preview | p95 at most 150 ms | Play/selection to first audible sound; state cache, device, buffer size. |
| Scrolling | p95 CPU frame work within 16.7 ms at 60 Hz | Measure active scrolling and visible rows; also report dropped frames. |

True audio latency may require loopback or external recording. Time until a
block is queued to CPAL measures only part of the path. An interactive library
before all background metadata is ready is valid when its state is clear.

## Measurement scenarios

1. Empty app, idle app, minimized app, library loaded without activity.
2. 10,000 and 100,000 metadata records; 1,000,000 as a later stress case.
3. Cold/warm startup and cold/warm first preview.
4. Search, sorting, scrolling, and rapid query changes.
5. WAV/FLAC/MP3/AIFF: short effects, long tracks, channel/rate variants.
6. Playback and rapid selection/seeking during scanning and analysis.
7. Pause, cancellation, device loss, shutdown with queued work.
8. Projects with many references and exports of many/large files.
9. Repeated opening/closing, project switching, and reindexing to detect leaks.

NAS/external disks are separate scenarios, not interchangeable with SSD results.
Identify synthetic fixtures explicitly. Request a representative collection or
use approved sample files; do not scan private folders by habit.

## Method

- Compare the same release profile, commits, machine, and power configuration.
- Record OS, CPU, logical cores, RAM, disk, audio device, backend, and sample rate.
- Record file count, total bytes, format/duration distribution, and index size.
- Define cold/warm precisely; do not manipulate system caches without a need.
- Repeat latency operations and report count, median, p95, and maximum.
- Measure CPU/RAM over time; one Task Manager screenshot is not a baseline.
- Report peak memory, disk I/O, queue sizes, and underruns alongside averages.
- Check whether renderer/drivers move work into GPU rather than eliminating it.
- Preserve reproducible commands and methods. Private raw data belongs in
  ignored `benchmarks/results/`; a sanitized summary can be committed.

Split scanning throughput into enumeration, metadata, decoding, waveforms,
hashing, and analysis. Files per second without content details are not
comparable. Bound background CPU and prioritize interaction/playback.
Idle operation and full analysis have different resource budgets.

## Prevent structural waste

No unbounded queues/caches, whole-library decoding into RAM, continuous repaint,
or tight polling without visible changes. Load relevant data, render visible
rows, and allow background work to pause, cancel, and leave defined state.

Profile a demonstrated bottleneck before optimization and preserve a simple
baseline. An algorithm microbenchmark does not prove lower application memory,
faster searches, or uninterrupted playback. A `perf` commit needs relevant
before/after evidence and preserved behavior.

## Benchmark report template

```text
Commit / build profile / date:
Machine / OS / audio device:
Library / scenario / cache state:
Method / commands / repetitions:
CPU / private bytes / working set / GPU / peak:
Latency median / p95 / maximum:
Disk I/O / underruns / queues:
Baseline comparison:
Limitations / raw data location / conclusion:
```
