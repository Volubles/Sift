# Audio export runtime

Original-byte exports require no encoder. Converted exports invoke the packaged
`export-tools/ffmpeg.exe` beside Sift; system PATH is never searched. The tool
runs only on explicit export/review requests, with bounded diagnostics, cooperative
cancellation, two codec threads and one filter thread. The playback decoder and
volume controls are not part of this pipeline.

Prepare a local Windows x64 bundle after building Sift:

```powershell
./scripts/prepare-export-tools.ps1 -OutputDirectory ./target/release
```

The script requires PowerShell 7 and HTTPS access during preparation. The prepared
application runs offline. An existing verified archive can be passed with
`-ArchivePath`; existing bundles are not overwritten. Copy the `export-tools`
directory together with Sift when testing a self-contained application folder.

The pinned BtbN snapshot is `autobuild-2026-10-14-13-03`, archive
`ffmpeg-n8.1.3-18-g46d899d7a0-win64-lgpl-shared-8.1.zip`, SHA-256
`38dd3fb973c2e10723489e2173135d60f569953e70a04b885446175f9e9380d6`.
The package identifies itself as LGPL version 3 or later, with GPL/nonfree parts
disabled. The script verifies its archive hash, version and reported license.
It installs the executable/DLLs, license texts, configuration, a file-hash manifest,
and exact FFmpeg/build-recipe source snapshots. `ffprobe` and `ffplay` are not
installed in the application bundle.

This is a local development bundle, not completed commercial distribution
qualification. The broad upstream build contains components beyond audio export.
Before redistributing a release, audit every enabled dependency's notices and
corresponding-source obligations, preserve replacement rights, and review the
selected formats' licensing/patent position. FFmpeg's source snapshot alone does
not cover source obligations for external libraries. A smaller audio-only build
can replace this bundle after equivalent codec, cancellation and output checks.

References: [FFmpeg legal guidance](https://ffmpeg.org/legal.html),
[FFmpeg license](https://ffmpeg.org/doxygen/trunk/md_LICENSE.html),
[build provider](https://github.com/BtbN/FFmpeg-Builds), and
[audio encoding options](https://ffmpeg.org/ffmpeg-codecs.html).
