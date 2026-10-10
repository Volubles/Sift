# Preview and waveform checkpoint

Implemented on Windows, 2026-10-09. The desktop now includes Play/Pause, Stop,
volume, a submitted-frame playhead, Space playback control, and click-to-seek
on a waveform. Selection starts the next sound when preview is already active;
initial selection only prepares a waveform and does not open an audio device.

## Architecture and limits

- A dedicated worker owns file decoding, rate conversion, and the CPAL stream.
  The device is opened lazily and released when paused, stopped, or finished.
- Stream recreation on seek/source changes prevents previously queued OS audio
  from being reused. Requests coalesce; cancellation invalidates old work.
- A 16,384-frame SPSC ring holds prepared stereo frames and generations. The
  data callback uses only the ring, stack values, and atomics: no I/O, logging,
  locking, allocation, or UI notification.
- Playback starts after 4,096 frames are prepared, or after a shorter clip's
  tail is flushed. Decoder work is prioritized when the buffer needs data.
- Mono is duplicated, stereo preserved, and 3-8 channels averaged to mono for
  this prototype. Output is float PCM at the device's default rate; other
  output formats and sources over 384 kHz are unsupported.
- Rubato converts rates in bounded chunks, bypassing equal rates and trimming
  startup delay/tail. UI position tracks submitted frames, not measured speaker
  latency. Waveform duration derives from decoded frames.
- Waveforms are reduced to at most 2,048 min/max bins with cooperative cancellation.
  A separate derived SQLite cache uses native path, size, modification time, and
  algorithm version. It bounds entries, peak payload, and database pages.
  Timestamp/size are change indicators, not cryptographic content identity.
- Source files are read only; cache failure falls back to analysis.

## Verification

The checks below were run with the private local verification suite and fixtures.
Those sources are not included in this source distribution.

- 26 default-feature tests passed, 23 headless tests passed, and Clippy with
  warnings denied and formatting checks passed.
- An explicitly run hardware test switches sources and starts a two-minute PCM
  stream while indexing proceeds in the same process. It passed with zero
  recorded underruns; it is ignored in normal hardware-independent test runs.
- Actual Windows WASAPI output callbacks consumed nonzero PCM from generated
  WAV, FLAC, MP3, and AIFF sources. The 44.1 kHz mono fixtures exercised conversion
  to the default output, pause stability, seeking, and reopening the output.
  All four final exercises reported zero underruns and a 1.200-second end time.
- First nonzero callback observations were approximately 34-48 ms in those
  debug runs. These are single-run internal observations, not audible latency
  or p95 measurements. Physical listening and device unplug/replug remain manual.
- SHA-256 checks showed unchanged generated sources after the device exercises.
- An actual release native window rendered the waveform and controls; captures
  were visually inspected. Mouse/keyboard interaction still needs hands-on QA.
- Invalid controls, missing sources, stream generations, cancellation, source
  preservation, seek, resampling duration/tail, cache invalidation, and corrupt
  cached peaks are covered by automated checks.

Raw audio, catalogs, screenshots, and logs are ignored scratch data. No private
sample collection was scanned and no microphone or system loopback was recorded.

## Output diagnostics

```powershell
cargo run --locked -- audio-check PATH_TO_GENERATED_AUDIO --exercise
```

The local hardware suite uses very low-volume disposable audio. Its invocation
is maintained with the private fixtures. The included audio-check command is
intended for short files and times out after ten seconds. Neither check proves
compatibility with every device/codec variant.

Projects, tags, export, native outgoing dragging, incremental indexing, and
similarity remain separate future checkpoints.
