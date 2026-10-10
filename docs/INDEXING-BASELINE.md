# 100,000-file incremental index measurement

Measured 2026-10-10 in release mode on Windows 10 Pro 10.0.19045, Intel
i9-13900K (32 logical processors), approximately 64 GiB RAM, local C: NTFS.
The storage was a fixed local volume; disk model/SSD classification was not
collected. No renderer, audio playback, or GPU measurement ran in this harness.

The fixture contained 100,000 actual generated WAV files across 100 directories:
mono PCM16, 8 kHz, 64 frames (8 ms), 172 bytes each, 17.2 MB audio bytes overall.
These tiny files test enumeration, catalogue size, and event handling, not a
representative sound-effects/codec distribution. No private library was scanned.

## Method

The private local indexing harness creates fixtures below
`.local/indexing-perf`, indexes them, repeats unchanged inventories three times,
starts the actual Windows watcher/index workers, then measures 20 individual
additions and one ten-file burst. Latency runs from file creation start to the
published synchronization result and a reader's correct catalogue count. It
includes the 200 ms settling interval; it does not include rendering or hearing
the preview. Every single addition inspected one audio file.

The comparison executable was the preceding Libraries release, archived before
this task as `target/release/sift-before-live-index.exe` (SHA-256
`BD11058DE0550F64F52060E6B4DD8128515302637729E1B6B66A4DE70F4C9D98`). Both
used the release profile, same machine, and same generated files. Initial builds
used fresh databases. Warm repeats reused their databases and OS caches; no
system cache was cleared. Separate compiler checks finished while fixtures were
being created, before the comparison scans.

## Results

| Operation | Observed result |
| --- | --- |
| Previous initial catalogue build | 14.760 s |
| Previous full metadata refresh, two warm repeats | 15.984 s and 15.125 s |
| New unchanged inventory, three repeats | 2.378 s, 2.505 s, 2.769 s; zero audio probes/sample writes |
| New initial build before directory-insert batching | 24.996 s |
| New initial build after directory-insert batching | 21.799 s, separate generated 100,000-file fixture, one run |
| Native single-file addition, 20 repetitions | median 234.864 ms, p95 237.834 ms, max 240.516 ms |
| Native burst of ten additions | 278.755 ms |
| Index-worker ready signal | 123.843 ms, before startup reconciliation finished |

Unchanged refreshes were about six times faster in this fixture, and small
updates did not enumerate the 100,000-file bank. The initial build remains
slower than the previous implementation. Timestamp collection, post-probe
stability checks, and the reconciliation inventory add work. Directory SQL is
now deduplicated within each batch, but its single repeat is not a controlled
component profile. Initial-import throughput remains an optimization target.

## Idle/resource observation

After changes settled, the harness blocked for 60 seconds on worker signals and
asserted that no spontaneous scan started. Host process sampling ran at about
1 Hz. The 57 samples strictly inside that interval covered 56.686 seconds;
later fixture-cleanup samples were excluded.

Private bytes averaged/maxed at 19.56 MiB; mean working set was 25.85 MiB.
There was no measurable increase in the process CPU-time counter during the
sampled interval. This is a headless index/watcher component result. It is not
the full Sift idle CPU/RAM qualification: no renderer, waveform/audio cache,
GPU, or other running Sift process was included. CPU counter resolution and
the shorter sampled window limit the claim.

## Reproduction conditions

Archive a verified preceding release executable under the comparison name above,
or omit it to measure the new implementation alone. The harness, fixtures, and
invocation instructions are maintained with the private local suite and are not
included in this source distribution. The method above specifies the generated
file population, warm/cold inventory conditions, settling interval, repetitions,
and completion criteria used for these results.

Resource sampling covers the measured process's private bytes, working set, and
CPU time during the marked idle interval. Fixtures use an owned temporary
directory inside the approved workspace and are removed when measurement ends.

Raw comparison output and process samples remain ignored under
`.local/indexing-perf/`. NAS/external drives, notification-loss storms, long
audio, sustained import, scrolling, and actual interaction latency require
separate measurements.
