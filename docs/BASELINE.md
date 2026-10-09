# First release resource baseline

Measured on Windows on 2026-10-09, with 100,000 synthetic metadata records,
the same 1120 x 720 browser, a local catalog, no interaction, and no audio.
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

## Reproduce

```powershell
cargo build --release --locked
.\scripts\measure-idle.ps1 -Executable .\target\release\sift.exe -Database PATH_TO_SYNTHETIC_CATALOG -Output .\benchmarks\results\idle.json -Seconds 60 -Renderer glow
```

The script starts and closes only its own test process, refuses existing report
files, and preserves the catalog/source files. Include GPU memory, interactive
latency, repeated starts, and a representative real library in future reports.
