# Audio export

The export engine creates independent files from captured sound selections.
Original mode copies exact bytes; WAV, AIFF, FLAC, MP3 and Ogg Vorbis read the
original source directly through the [packaged runtime](EXPORT-TOOLS.md).
Rate, channels, PCM depth, FLAC compression and lossy quality are explicit typed
settings. FLAC compression changes size/encoding work, not decoded sample quality
at the selected integer depth. Unsupported MP3 rates/channel counts are rejected
rather than silently converted to different settings.

Review resolves up to 128 references from the same captured query, sort order,
folder scope and marks used elsewhere in Sift. It validates all sources, names,
settings and the existing destination before creating output. Source locators
remain native paths; output names are independently validated for traversal,
Windows reserved names, controls and trailing spaces/dots. Source paths share a
1 MiB budget; destination ancestry is bounded to 128 directories.

Keep both plans unique names; Skip preserves existing files; Replace is explicit
and refuses every selected source and hard-link alias. Internal duplicate names
under Replace fail review. Links/reparse targets and destination ancestors are
rejected. Open source handles and file identities persist through a reviewed
plan; Windows source handles deny writing/deletion and destination handles deny
directory renaming. Review invalidation/cancellation releases the plan on its owner.

One background owner serializes selection loading, review and export. Its command
queue has one slot, events have four slots, and job revisions reject stale UI
results. It executes one file at a time; byte copies use a 64 KiB buffer, while
encoding streams through a child process with bounded stdout/stderr readers.
Idle workers wait for commands. Cancellation/shutdown stops the owned child,
joins readers, and preserves completed exports.

Files render into uniquely created temporary files in the destination. After
encoding, actual rate/channels are checked through the audio probe. Sources are
revalidated and output is flushed before publication. Keep both/Skip use
non-replacing publication; a late collision chooses a new free name or reports a
skip. Replacement is revalidated immediately before publication. Cleanup errors
are reported with the failed stage. Completed, skipped, failed and cancelled
files are separate per-file outcomes; partial completion is never whole-batch
success. Filesystem operations already blocked in the OS, especially network
reads, have cooperative rather than guaranteed immediate cancellation.

Converted output currently carries audio only. Source metadata, cue/loop markers,
embedded art, BWF chunks and Sift's manual tags are not promised across conversion;
Original mode preserves every source byte. Lossy encoding cannot recover detail
lost earlier, and changing rate/depth/channels can change samples even for a
lossless target codec. Conversion and reindexing never change source bytes.
