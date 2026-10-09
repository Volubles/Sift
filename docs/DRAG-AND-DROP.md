# Native file drag checkpoint

## Contract and ownership

The Windows adapter offers original file references through OLE `IDataObject`
and `IDropSource`. It passes only `DROPEFFECT_COPY` to `DoDragDrop` and advertises
the same preferred effect. Sift does not move, delete, rename, transcode, or stage
copies of source audio. The destination application owns copying, conflict
prompts, cancellation after acceptance, and destination errors.

An accepted drop means **the target accepted the copy offer**, not that every
destination file was published successfully. Do not report it as completed
export. Application-managed export with explicit conflict planning and per-file
outcomes remains a separate feature.

The desktop starts dragging from a library row or the selected sound's
**Drag copy to folder** control. Hold the left mouse button, move into an Explorer
folder, and release. Escape cancels. Clicks continue to select sounds; secondary
button dragging does not start this operation. Busy, cancellation, rejection,
source failures, and handoff have visible states. After the result, egui clears
the drag ID and pointer state because OLE receives the final mouse release on
its own thread. A deterministic interaction test covers click/drag separation
and reusing the row after that reset.

The adapter is available through the `native-drag` feature, which desktop enables
by default. It supports multiple references internally; the current desktop
selects and drags one sound. Other platforms do not claim outgoing native drag
support yet.

## Threads, bounds, and native safety

- One dedicated drag worker; one request and one result slot. A second request
  is refused until the first outcome is consumed. Idle workers block.
- At most 128 paths, 32,767 UTF-16 units per path, and 1 MiB of file-reference
  payload. Only path references are transferred; audio is not loaded into RAM.
- Source existence, regular-file type, and reparse/link checks happen on the
  worker. Validation is a snapshot: a third-party target opens the path later.
- The UI relinquishes mouse capture; OLE takes its own capture on the worker.
  A late request after left-button release is cancelled. Escape cancels the
  native gesture. Shutdown sets a cancellation flag, posts a message only to
  the owned worker to wake its modal loop, disconnects queues, and joins it.
- Each transfer initializes an STA with `OleInitialize`. COM objects stay on
  that thread and are released before the matching `OleUninitialize`.
- Each `GetData` creates independent movable `HGLOBAL` storage. A local guard
  frees unpublished allocations; the receiving target owns successful media
  and releases them through `ReleaseStgMedium`.

`windows` and `windows-core` are pinned to 0.62.2 (MIT OR Apache-2.0), already
present in the previous dependency graph. Enabled Win32 APIs cover COM/OLE,
global memory, file references, mouse capture, and the worker message queue.
There is no new framework or runtime. `unsafe_code` is denied throughout Sift
and allowed only in `src/platform/windows_drag.rs`, including its native tests;
`unsafe_op_in_unsafe_fn` remains denied. Safety comments describe pointer and
ownership contracts at the boundary. COM method inputs are validated before use.

## Paths

`CF_HDROP` contains a wide `DROPFILES` header and a double-null-terminated list
of absolute UTF-16 paths. Lossy display strings never become file locators.
Windows' own `DragQueryFileW` verifies the encoded references in tests, including
spaces, Unicode, long paths, and unpaired UTF-16 surrogates.

Catalog paths are canonical extended locators. In the local Shell test, offering
`\\?\C:\...` directly was accepted but produced no files. The adapter therefore
offers conventional drive or UNC paths. It rejects parent/current components,
reserved device names, alternate streams, trailing spaces/dots, and device or
volume namespace paths that cannot be represented safely without changing their
meaning. The catalog locator is unchanged. A greater-than-260-character source
with ordinary components was successfully copied by the tested Shell target.
UNC encoding is tested; an actual network share is not yet tested.

## Verification and remaining checks

Automated tests cover selection bounds, absolute paths, missing sources,
directories, shutdown with an unread outcome, independent native media,
format/aspect/index/medium validation, UTF-16 preservation, payload limits,
safe path conversion, Escape, modifiers, copy-only negotiation, and an actual
OLE cancellation with a worker-queue wake.

The explicit Windows Shell integration test invokes the actual folder
`IDropTarget`, obtained from `IShellItem::BindToHandler(BHID_SFUIObject)`. It copies
three generated PCM WAV files to a disposable folder, checks complete byte-for-
byte destination contents, and verifies unchanged originals. Fixtures include
spaces, Unicode, canonical extended locators, and a source path over 260
characters. No private library is scanned and no recording is made.

```powershell
cargo test --locked --lib platform:: -- --test-threads=1
cargo test --locked --lib shell_target_copies -- --ignored --test-threads=1
```

This proves native file-data interoperability and actual Shell copying. It does
not prove the full Sift mouse gesture into an Explorer window. Manual Explorer
checks remain: release and repeated gestures, Escape, Shift/Ctrl/Alt, existing
destination names, same-folder drops, missing sources, and playback while
dragging. Other DAWs/applications, network shares, denied permissions, full disks,
and batch failure outcomes require their own runtime evidence.

The integrated desktop passes 34 default-feature tests, 23 CLI/core tests with
default features disabled, formatting, and warning-free Clippy for both those
configurations. Seven adapter tests also pass with only `native-drag` enabled.
The explicit Shell target test above passes separately. The default test command
intentionally skips that Shell integration test and the existing real-audio-device
test. A release build and native desktop capture with the new drag control pass;
the capture was visually inspected. These checks do not establish audible output
or a successful interactive Explorer gesture.

The integrated release resource observation used a 100,000-row synthetic catalog
for 60 seconds and recorded about 82 MiB private memory with no measurable
process CPU-time increase. Exit diagnostics recorded input activity, so this is
not a controlled idle claim. See [resource baseline](BASELINE.md) for the method,
raw-record context, and unmeasured active-drag/playback scenarios.

## Primary references

- [Microsoft: DoDragDrop](https://learn.microsoft.com/en-us/windows/win32/api/ole2/nf-ole2-dodragdrop)
- [Microsoft: Shell clipboard formats](https://learn.microsoft.com/en-us/windows/win32/shell/clipboard)
- [Microsoft: copy/move effects](https://learn.microsoft.com/en-us/windows/win32/com/dropeffect-constants)
- [Pinned windows-rs source](https://docs.rs/windows/0.62.2/windows/Win32/System/Ole/index.html)
