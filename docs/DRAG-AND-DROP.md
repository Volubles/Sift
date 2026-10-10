# Native file dragging

## Contract and ownership

The Windows adapter offers original file references through OLE `IDataObject`
and `IDropSource`. It passes only `DROPEFFECT_COPY` to `DoDragDrop` and advertises
the same preferred effect. Sift does not move, delete, rename, transcode, or stage
copies of source audio. The destination application owns copying, conflict
prompts, cancellation after acceptance, and destination errors.

An accepted drop means **the target accepted the copy offer**. Destination
publication and any copying failures belong to that target. Application-managed export with explicit conflict planning and per-file
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
modal loop. The reset lets the row accept another click or drag after native completion.

The adapter is available through the `native-drag` feature, which desktop enables
by default. It supports multiple references internally; the current desktop
selects and drags one sound. Other platforms do not claim outgoing native drag
support yet.

Collections add an internal egui drag phase: a held drag within Sift can add a
reference to a collection name. Native copying starts only when the primary
drag leaves the client area. The copy-only/self-drop safeguards then apply;
returning inside after native handoff ends quietly. See [Collections](COLLECTIONS.md).

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

`windows` and `windows-core` are pinned to 0.62.2 (MIT OR Apache-2.0). Enabled Win32 APIs cover COM/OLE,
global memory, file references, mouse capture, and the owned UI message window.
The adapter uses scoped unsafe code in its OLE/host modules;
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

## Qualification

Local verification covers bounds, path validation, native media ownership,
clipboard formats, copy-only effects, cancellation, deferred owner-thread
dispatch, self-drops, retry, and unread-result shutdown. An explicit Windows
Shell target check copied three generated WAV files into a disposable directory
with byte-identical destinations and unchanged originals. Fixtures included
Unicode, spaces, extended locators, and a source longer than 260 characters.

The harness and fixtures are maintained privately. Native target interoperability
and window captures do not establish complete interactive Explorer behavior.
Physical repeated gestures, Escape/modifiers, conflicts, same-folder drops,
missing sources, and playback during dragging still need broader qualification.
DAWs, network shares, denied permissions, and full disks have separate limits.

OLE starts after the current egui/winit callback returns. This avoids entering a
nested message loop while application/event-handler state is borrowed; Windows
can deliver redraw messages during that loop. The OS supplies drag cursor
feedback. A custom floating file thumbnail is not implemented.

Resource observations are recorded in [the desktop baseline](BASELINE.md).

## Primary references

- [Microsoft: DoDragDrop](https://learn.microsoft.com/en-us/windows/win32/api/ole2/nf-ole2-dodragdrop)
- [Microsoft: Shell clipboard formats](https://learn.microsoft.com/en-us/windows/win32/shell/clipboard)
- [Microsoft: copy/move effects](https://learn.microsoft.com/en-us/windows/win32/com/dropeffect-constants)
- [Pinned windows-rs source](https://docs.rs/windows/0.62.2/windows/Win32/System/Ole/index.html)
