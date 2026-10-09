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
button dragging does not start this operation. Preparation and active dragging
have visible states. Acceptance, Escape, and unsuitable destinations end quietly
without a persistent banner or a notification timer. Actual source/native
failures remain visible with **Dismiss**, or clear when a new drag starts.
After the result, egui clears
the drag ID and pointer state because OLE consumes the final release in its
modal loop. A deterministic interaction test covers click/drag separation
and reusing the row after that reset.

The adapter is available through the `native-drag` feature, which desktop enables
by default. It supports multiple references internally; the current desktop
selects and drags one sound. Other platforms do not claim outgoing native drag
support yet.

## Threads, bounds, and native safety

- One dedicated preparation worker; one request and one preparation result slot.
  A second request is refused until the first final outcome is consumed. Idle
  workers block. Only file validation and UTF-16 payload preparation run there.
- At most 128 paths, 32,767 UTF-16 units per path, and 1 MiB of file-reference
  payload. Only path references are transferred; audio is not loaded into RAM.
- Source existence, regular-file type, and reparse/link checks happen on the
  worker. Validation is a snapshot: a third-party target opens the path later.
- A private message-only HWND belongs to the Sift window's input thread. Posting
  its start message defers the modal OLE call until the current egui/winit
  callback has returned. Native objects stay on that thread; Rust `Rc` prevents
  moving the host/coordinator across threads. No input queues are attached and
  no mouse/keyboard input is injected into other apps.
- At native start the UI relinquishes capture and OLE takes its own capture on
  the same input thread. Late starts after left-button release or while Escape
  is pressed are cancelled. Preparation supports Escape, mouse release, and
  **Cancel file drag**; native OLE handles Escape during the actual gesture.
- On release, the source checks the window under the cursor. A window owned by
  this Sift process, or no valid window, cancels before `IDropTarget::Drop` can
  run. This prevents winit's generic incoming file handler from accepting a
  self-drop. External targets still negotiate copy support; another application
  may accept the file for import. A target that does not accept it ends quietly.
- Shutdown cancels the pending job, disconnects queues, joins preparation,
  clears the HWND callback pointer, and destroys only the owned message window.
  A callback retains its thread-local state through nested events. Final native
  outcomes have one queue slot, wake the UI, and clear the busy state on receipt.
- Each native transfer balances `OleInitialize`/`OleUninitialize` on the input
  thread. Windows owns the modal gesture and pumps native messages; background
  decoding/indexing continue, though view redraw timing remains unqualified.
- Each `GetData` creates independent movable `HGLOBAL` storage. A local guard
  frees unpublished allocations; the receiving target owns successful media
  and releases them through `ReleaseStgMedium`.

`windows` and `windows-core` are pinned to 0.62.2 (MIT OR Apache-2.0), already
present in the previous dependency graph. Enabled Win32 APIs cover COM/OLE,
global memory, file references, mouse capture, and the owned UI message window.
There is no new framework or runtime. `unsafe_code` is denied throughout Sift
and allowed only in `src/platform/windows_drag.rs` and its native host submodule,
including their native tests;
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

The desktop passes 38 default-feature tests, formatting, and warning-
free Clippy. The earlier CLI/core checkpoint passed 23 tests with default
features disabled. Native tests include an actual owned HWND and an invalid
window handle: both release decisions cancel before any target drop. The prior
adapter-only configuration passed ten checks.
The explicit Shell target test above passes separately. The default test command
intentionally skips that Shell integration test and the existing real-audio-device
test. A release build and native desktop capture with the new drag control pass;
the capture was visually inspected. These checks do not establish audible output
or a successful interactive Explorer gesture.

Before the input-thread repair, the release observation used a 100,000-row synthetic catalog
for 60 seconds and recorded about 82 MiB private memory with no measurable
process CPU-time increase. Exit diagnostics recorded input activity, so this is
not a controlled idle claim. See [resource baseline](BASELINE.md) for the method,
raw-record context, and unmeasured active-drag/playback scenarios.

## Input-thread repair

The original interactive attempt on Windows stayed in "Dragging a copy" without
native feedback, and Escape did not finish it while other Sift controls remained
usable. Native file-data/Shell tests had not exercised that gesture. The original
implementation ran OLE on an unrelated background input queue.

The repair keeps filesystem preparation on its worker and posts native startup
to a private HWND on the creator's UI/input thread. It also avoids calling a
modal message loop while egui or winit's event handler is borrowed: the pinned
winit Windows runner can immediately dispatch `RedrawRequested` during nested
messages. A native HWND test verifies deferred delivery on the owning thread,
cancellation after startup is queued, final delivery, cleared busy state, and
unchanged sources. Additional regressions cover off-thread preparation, caller-
thread dispatch, cancellation before a late start, retry, and unread shutdown.

The Windows automation helper could not start (`failed to write kernel assets`,
OS error 3), so the repaired physical Sift-to-Explorer gesture is not yet
automatically verified. The default release binary was rebuilt and the native
window smoke check passes. Manual copy, repeated drag, Escape, and modifiers
remain required; the source UI should now show native copy cursor feedback
when a target accepts the offer. A custom floating file thumbnail is not
implemented.

## Primary references

- [Microsoft: DoDragDrop](https://learn.microsoft.com/en-us/windows/win32/api/ole2/nf-ole2-dodragdrop)
- [Microsoft: Shell clipboard formats](https://learn.microsoft.com/en-us/windows/win32/shell/clipboard)
- [Microsoft: copy/move effects](https://learn.microsoft.com/en-us/windows/win32/com/dropeffect-constants)
- [Pinned windows-rs source](https://docs.rs/windows/0.62.2/windows/Win32/System/Ole/index.html)
