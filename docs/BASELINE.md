# Renderer and audio resource measurements

Measured on Windows on 2026-10-09, with 100,000 synthetic metadata records,
the same 1120 x 720 browser, a local catalog, no deliberate interaction, and no audio.
CPU: Intel Core i9-13900K, 32 logical processors. The system exposes NVIDIA
RTX 5080, Intel UHD 770, and Parsec display adapters; GPU selection/memory were
not independently measured. These are single-run baselines, not a population
benchmark or a latency claim.

| Renderer | Mean private MiB | Peak private MiB | Mean working-set MiB | CPU % of entire machine | Window appearance, seconds |
| --- | --- | --- | --- | --- | --- |
| Original WGPU configuration | 476.81 | 476.84 | 264.72 | 0.000 at counter resolution | 0.359 |
| WGPU with DX12 override | 416.17 | 416.35 | 240.34 | 0.224 | 0.101 |
| egui Glow/OpenGL comparison | 133.72 | 133.80 | 138.35 | 0.043 | 0.315 |

Each run settles for five seconds and samples process private bytes/working set
every two seconds for approximately 60 seconds. CPU is process CPU-time delta
divided by elapsed time and 32 logical processors. Native window appearance is
not proof of interactive readiness. Shader/OS caches were not flushed; startup
figures are observations, not comparable cold/warm percentiles.

The original binary was built at `2fe29bc`. The Glow comparison used the same
view with a working-tree change enabling/selecting Glow; raw JSON records the
dirty state. Raw data is ignored under `benchmarks/results/`.

## Decision

Use Glow by default: its measured private-memory cost is approximately 72% below
the original WGPU configuration on this system and fits the proposed idle
private-memory/CPU budgets. This does not establish release budgets during
playback, indexing, active scrolling, or on another GPU/driver.

Keep WGPU available with the `wgpu-renderer` feature. Select it for diagnostics
with `SIFT_RENDERER=wgpu`; an unsupported selection fails explicitly. This is a
renderer decision within egui, not a change to the core/UI framework.

## Post-preview observations

The preview build also fits the private-memory target in the observed runs:
approximately 77.21 MiB mean private bytes and 74.59 MiB working set in a
30-second no-input repeat. Its exit diagnostic recorded 37 startup/UI frames,
zero input events, and idle audio state; process CPU-time delta was zero at
counter resolution. This shorter repeat does not qualify the full 60-second
idle budget or audible latency.

Two other preview observations reported 0.676%/0.813% machine CPU. The latter
recorded 9,754 input events and 2,748 UI frames, so it was active interaction,
not idle. The initial run lacked activity instrumentation. Neither observation
qualifies idle CPU; both remain part of the recorded evidence. Input was
not prevented during the original renderer baselines either; comparisons remain
local observations requiring controlled repetition on representative hardware.

## Post-drag observation

The integrated native-drag release build was observed with the same 100,000-row
synthetic catalog for 60.37 seconds after a five-second settle. Mean private
memory was 82.08 MiB (82.19 MiB peak), and mean working set was 78.58 MiB. No
process CPU-time increase was measurable in the sampling interval. The native
window appeared after 0.089 seconds; this is not interactive-readiness timing.

Exit diagnostics recorded 123 frames, 307 input events, and idle audio state.
Input was not prevented, and those counts include startup/settle. This run is
therefore an activity-qualified resource observation, not a certified no-input
idle benchmark or a paired before/after comparison. GPU memory, active dragging,
Shell copy memory, and simultaneous playback resource costs were not measured.
The raw report records the adapter build and uncommitted desktop integration
changes and is ignored under `benchmarks/results/`.

## Collections observations

The Collections release was observed with the same 100,000-row synthetic catalog
and an empty durable collection store. A 60.37-second run recorded 86.15 MiB mean
private bytes (87.11 MiB peak), 80.53 MiB working set, and 1.040% machine CPU.
Its diagnostics recorded 3,570 frames and 13,726 input events, so this was active
interaction rather than controlled idle operation.

A separate 30.18-second repeat recorded 81.82 MiB mean private bytes (81.83 MiB
peak), 79.10 MiB working set, and no measurable CPU-time increase during sampling.
Exit diagnostics recorded 124 frames and 246 input events, including startup;
input was not prevented. This shorter repeat is still an activity-qualified
observation, not proof of a full no-input idle budget. Both reports identify
the storage build with uncommitted desktop integration changes. GPU memory,
large saved collections, active dragging, and concurrent playback were not
resource-qualified. Raw results remain ignored under `benchmarks/results/`.

## Reproduction conditions

```powershell
cargo build --release --locked
```

The measurement script and synthetic catalog are maintained privately. Its
invocation is recorded with the local verification suite and is not included
in this source distribution. It starts/closes only its own process, refuses
existing reports, and preserves catalog/source files. The method above records
sampling intervals, startup/settling conditions, CPU normalization, and activity
limitations. GPU memory, interactive latency, repeated starts, and representative
real libraries remain qualification gaps.
