# Product overview

Sift browses existing sound effects, samples, and music on Windows. Its main
workflow is finding a sound, previewing it, keeping a reference, and copying the
original file into another application or folder. Source audio stays in place.

## Current workflow

Libraries group one or more source folders under a name. A library or folder
selection scopes the sound table and search. Folder expansion and sound scope
are separate actions; breadcrumbs provide a path back through ancestors.

The selected sound has waveform, playback, seeking, volume, auto-play, and
metadata controls. Global favorites provide quick access to saved references.
Sound lists support multiple marks, keyboard navigation, and an active preview
distinct from the marked set. Auto-play starts enabled. See [Selection](SELECTION.md).
Collections keep named selections without duplicating audio. Names and saved
membership changes persist automatically, with visible save failures and retry.

Windows folder notifications update indexed metadata after changes settle.
Pausing live updates retains data; resuming checks for missed changes.
Missing and unavailable sources remain distinguishable from empty results or
unsupported audio.

External dragging offers copying to Explorer. The destination owns copying
progress and conflicts. Internal dragging adds references to collections.

## Data preservation

Library registration, collection edits, and favoriting leave source names,
contents, and locations unchanged. Removing saved items removes references.
Durable registrations and saved selections are stored separately from the
rebuildable catalog and waveform cache.

Cancelled or incomplete inventories preserve previous cached rows. Migration
errors preserve durable data. Missing sources retain saved references, and
ambiguous locations are not matched automatically to replacement sounds.

## Supported scope

The application is a Windows prototype with a CLI-only configuration sharing
the same core. WAV, FLAC, MP3, and AIFF metadata are recognized; codec variants,
devices, external drives, and network shares have individual qualification limits.
The core workflow works offline.

Collection adds/removals and outgoing dragging currently act on one sound.
Group actions, collection folders, user-controlled item
ordering, tags, application-managed export, portable saved
selections, and explicit source relinking are not implemented. Similarity search
is optional future work. Recording, audio editing, cloud synchronization, and
a timeline are outside the current product scope.

Responsiveness and bounded resource use are design goals. Existing observations
and remaining qualification are described in [Performance](PERFORMANCE.md).
The feature contracts are documented in [Libraries](LIBRARIES.md),
[Collections](COLLECTIONS.md), [Favorites](FAVORITES.md), [Preview](PREVIEW.md),
and [Drag and drop](DRAG-AND-DROP.md).
