# Product requirements

## Goal and priorities

Sift is a fast, resource-efficient desktop application for finding, previewing,
organizing, and exporting sounds from large local collections. Sound effects
are a primary use case; samples and complete music files must also work.
Sononym is a functional reference. Sift has its own implementation and design.

Prioritize preserving audio and user data, a responsive workflow, low CPU/RAM
and disk activity, and clear features. Performance and reliability are part
of a feature's quality.

## Agreed foundations

- Rust is the primary language. The application works locally without accounts
  or network access.
- Collections collect references to sounds; a sound can belong to several collections.
- Dragging from the application to an external folder copies the original file.
- Tagging, collection operations, and export do not modify library originals.
- AI categorization is outside the planned core product.
- Similarity search is a possible later step; the core workflow works without it.
- Small, coherent checkpoint commits are the standard workflow.
- Documentation, product UI, and all user-visible content are in English.

Windows is the proposed first platform, based on Benjamin's working environment.
A portable core should allow further platforms without promising support before
verification. Publishing, licensing, and final visual design remain open.

## Libraries and indexing

Users can create named libraries containing one or more existing source folders
without importing or copying audio. The library sidebar lists banks above a
folder browser; the central table shows the selected bank, or a selected folder
and its descendants. `All sounds` restores the bank scope. Search stays scoped,
and switching to Collections preserves the current library view. Removing a
bank or source registration must not delete its audio. Durable registrations
remain separate from rebuildable searchable metadata.

The Clear tree layout uses aligned chevron/icon/name columns, hierarchy guides,
distinct hover/selection/focus, and cached source counts or offline status.
Chevron clicks open a branch; row clicks or Enter select its sounds. Arrow keys
navigate without changing the selected sound scope. Breadcrumbs show the native
ancestor chain and provide parent navigation. Library rows use their content
height up to a bound, leaving the remaining sidebar to Folders. Subfolder counts
are outside this checkpoint; do not add per-frame count queries for decoration.

An initial scan shows genuine progress, errors, and pause/cancel controls.
Later scans primarily process changed files. Define behavior for symbolic-link
cycles, repeated roots, and overlapping roots. Changes during a scan must not
produce a false completed state.

Registered source folders are watched for additions, writes, renames, moves,
and removals. Small changes update the index without rescanning ordinary
unchanged audio. Coalesce bursts, recover explicitly from notification loss,
show pending work, preserve view state, and allow pausing/resuming live updates.
Offline roots retain cached metadata and manually saved work. Validate the
100,000-file scenario with real generated files before claiming responsiveness.

Initial format targets are WAV, FLAC, MP3, and AIFF. Verify actual codec variants,
bit depths, and metadata in the prototype. Reading a container is not equivalent
to complete metadata or playback support. Report unsupported, corrupt, and
unavailable files clearly and continue with the rest of the library.

Metadata can include name, location, duration, size, format, channels, sample
rate, meaningful bit depth, manual tags, and favorites. Derived analysis records
its algorithm version and incomplete state. Music-specific BPM/pitch are later
features and must support unknown values.

## Finding and previewing sounds

- Search names, paths, and tags; filter and sort relevant metadata.
- Handle large results with stable sorting and bounded data loading.
- Support keyboard navigation, multi-selection, and clear active-sound state.
- Show a reusable waveform with play/pause, stop, seek, and volume controls.
- Rapid selection changes stop the old preview and prioritize the new selection.
- Stream or buffer long music files with bounded memory, including waveform work.
- Device interruption must produce a clear state and support recovery.

Background analysis must not block search. A delayed response to an old query
must not replace a newer result. Distinguish no matches, pending search, and
search failure in the UI.

## Collections and saving

Favorites are global saved references with a star on Library and Collection
sound rows and a dedicated rail view. Their status is shared across these views.
The Favorites view has independent search, bounded results, source availability,
preview, and copy-only dragging. Changes save automatically with visible failure
and retry; removing a favorite preserves source audio and collection membership.
Favorites survive restart, detached libraries, and rebuilding the catalog.

Collections can be created, renamed, opened, and saved. They contain selected
sounds, ordering, virtual folders, and optional export names and notes.
Deleting a collection or item removes its references.

Accepted edits are saved transactionally and survive restart. Show pending or
failed saves and preserve unsaved changes for retry. Cache cleanup and full
reindexing must not remove collections, favorites, or manual tags.

Show correct status when a source moves, changes, or disappears. Users can
reconnect a root or select a replacement source. Ambiguous matches must not
silently attach a collection to the wrong sound. Making a collection self-contained
by copying sounds into its folder is a later export option; ordinary collections
must not duplicate the library.

## Copying, export, and drag-and-drop

Users can drag one or more collection sounds into Explorer and receive copies with
unchanged audio content. Exporting to a chosen folder is a complementary
workflow. Library originals and collection references remain intact.

Plan destination paths before writing. Offer skip, unique naming, or explicit
replacement for conflicts; default to preserving existing files. Handle reserved
Windows names, Unicode, long paths, permissions, full disks, and missing sources.

Cancellation reports files already copied and files remaining. Clean up only
temporary files created by Sift. Do not report an entire batch as successful
after partial failure. Test native dragging in Explorer; validate DAW/other
application support for each actual target.

## Later features

- Similarity search using precomputed acoustic descriptors. Evaluate quality for
  sound effects and music, and present similarity as a ranking.
- Exact duplicates using content hashes; acoustically close duplicates are a
  separate result with different confidence. No automatic deletion.
- Richer tagging, saved queries, export naming, and analysis filters.
- Portable collection descriptions and package exports with an explicit schema.

Semantic search, cloud sync, recording, plugin formats, audio editing, a
timeline/DAW, and full Sononym parity are outside the first delivery. Expanding
into these areas requires a concrete product decision.

## Proposed delivery stages and acceptance

| Stage | Delivery | Acceptance |
| --- | --- | --- |
| 0 | Repository, instructions, specification | Documents distinguish decisions, proposals, and actual status. |
| 1 | Technical prototype | Large virtual list, playback, waveform, and outgoing copy drag work on Windows; CPU/RAM are measured. |
| 2 | Libraries and search | Register, scan, search, and restart; handle changed and corrupt files. |
| 3 | Collections | Save references, ordering, and folders; preserve originals; show missing sources. |
| 4 | Copying and robustness | Verify multi-selection, collisions, cancellation, and failures with unchanged sources. |
| 5 | Performance and distribution | Verify representative measurements, recovery, and a Windows package. |
| 6 | Optional similarity search | Demonstrate useful rankings and acceptable resource cost. |

Stages describe direction, not authorization to implement everything at once.
Split each stage into natural implementation checkpoints when carrying it out.

## References

- [Sononym](https://www.sononym.net/)
- [Sononym projects](https://www.sononym.net/docs/manual/projects/)
- [Sononym similarity search](https://www.sononym.net/docs/manual/similarity-search/)
