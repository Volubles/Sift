# Audio preview and waveforms

The selected sound has Play/Pause, Stop, volume, auto-play, a waveform, and source
details. Auto-play starts a newly selected sound. With auto-play disabled,
selection stops the previous preview and prepares the new waveform without
opening an output device. Clicking the waveform plays from that position.

Space toggles playback when text fields and the folder tree do not own keyboard
focus. Missing sources and metadata errors disable preview and remain visible.
Changed sources retain their saved references while selected metadata is checked.

## Selection ownership

`desktop::preview::Preview` owns the selected reference, availability, source
revision, waveform, output state, volume, and auto-play setting. It queues source
inspection on the catalog read worker and owns its waveform/player interfaces.
Source, waveform, and playback generations reject late replies after selection
or file changes. An observed write invalidates the preview even when size and
timestamps match the previous metadata.

## Playback

A dedicated worker owns decoding, rate conversion, and the CPAL stream. The
device opens lazily and is released when paused, stopped, or finished. Seeking
or changing source recreates the stream to discard previously queued OS audio.
Requests coalesce and cancellation invalidates old work.

A 16,384-frame SPSC ring holds stereo frames and generations. Playback starts
after 4,096 prepared frames, or after the tail of a shorter clip is flushed.
The callback uses preallocated frames, stack values, and atomics, with no file
I/O, logging, locking, allocation, or UI notification.

Mono is duplicated, stereo preserved, and 3-8 channels averaged to mono. Output
is float PCM at the device's default rate; other output formats and sources over
384 kHz are unsupported. Rubato converts rates in bounded chunks, bypassing
equal rates and trimming startup delay and tail.

The playhead tracks submitted frames, not measured speaker latency. Visible
playback requests a repaint about every 33 ms; idle preview has no repaint timer.

## Waveforms

Analysis reduces decoded audio to at most 2,048 min/max bins with cooperative
cancellation. A separate derived SQLite cache uses native path, size,
modification time, and algorithm version. Entries, peak payloads, and database
pages have bounds. Cache failure falls back to analysis. Audio sources are read
only; no preview operation rewrites or transcodes them.

## Qualification

Local verification covers decoding, seeking, cancellation, rate-conversion
duration/tail, missing sources, corrupt cache entries, source preservation,
generation changes, and observed writes. Actual WASAPI callbacks consumed
nonzero PCM from generated WAV, FLAC, MP3, and AIFF fixtures; final exercises
reported zero underruns and a 1.200-second end time. Their first nonzero callbacks
were approximately 34-48 ms in debug mode, measured once per exercise.

Those callback observations do not measure audible latency or percentiles.
Physical listening, device unplug/replug, broader codec/output variants, and
interaction under sustained load need further qualification. Test sources and
fixtures are maintained privately. Resource observations are in
[the renderer/audio baseline](BASELINE.md).

## Output diagnostics

```powershell
cargo run --locked -- audio-check PATH_TO_GENERATED_AUDIO --exercise
```

This included command exercises short files and times out after ten seconds.
It reports callback counters; it does not certify device or codec compatibility.
