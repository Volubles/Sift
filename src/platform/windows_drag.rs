//! The only unsafe boundary: apartment-local OLE objects and owned HGLOBAL data.
#![allow(unsafe_code)]

use std::mem::ManuallyDrop;
use std::os::windows::ffi::OsStrExt;
use std::path::{Component, Path, Prefix};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use windows::Win32::{
    Foundation::{
        DATA_S_SAMEFORMATETC, DRAGDROP_S_CANCEL, DRAGDROP_S_DROP, DRAGDROP_S_USEDEFAULTCURSORS,
        DV_E_DVASPECT, DV_E_FORMATETC, DV_E_LINDEX, DV_E_TYMED, E_INVALIDARG, E_NOTIMPL,
        ERROR_SUCCESS, GetLastError, GlobalFree, HGLOBAL, HWND, OLE_E_ADVISENOTSUPPORTED, POINT,
        S_OK, SetLastError,
    },
    System::{
        Com::{
            DATADIR_GET, DVASPECT_CONTENT, FORMATETC, IAdviseSink, IDataObject, IDataObject_Impl,
            IEnumFORMATETC, IEnumSTATDATA, STGMEDIUM, STGMEDIUM_0, TYMED_HGLOBAL,
        },
        DataExchange::RegisterClipboardFormatW,
        Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock},
        Ole::{
            CF_HDROP, DROPEFFECT, DROPEFFECT_COPY, DROPEFFECT_NONE, DoDragDrop, IDropSource,
            IDropSource_Impl, OleInitialize, OleUninitialize,
        },
        SystemServices::{MK_LBUTTON, MODIFIERKEYS_FLAGS},
        Threading::GetCurrentProcessId,
    },
    UI::{
        Input::KeyboardAndMouse::{
            GetAsyncKeyState, GetCapture, ReleaseCapture, VK_ESCAPE, VK_LBUTTON,
        },
        Shell::{CFSTR_PREFERREDDROPEFFECT, DROPFILES, SHCreateStdEnumFmtEtc},
        WindowsAndMessaging::{GetCursorPos, GetWindowThreadProcessId, WindowFromPoint},
    },
};
use windows_core::{BOOL, HRESULT, Ref, implement};

use super::{DragError, DragOutcome, Sources};

const MAX_PATH_UNITS: usize = 32_767;
const MAX_PAYLOAD_BYTES: usize = 1024 * 1024;

mod host;
pub(super) use host::DragHost;

fn release_pointer_capture() -> Result<(), DragError> {
    // SAFETY: called on the UI thread; GetCapture returns only that thread's
    // capture. OLE acquires its own capture on this same input thread.
    unsafe {
        if !GetCapture().is_invalid() {
            ReleaseCapture().map_err(native_error)?;
        }
    }
    Ok(())
}

struct Apartment;

impl Apartment {
    fn enter() -> windows_core::Result<Self> {
        // SAFETY: all COM objects are created/dropped on this initialized thread
        // before this guard. S_FALSE also requires a matching OleUninitialize.
        unsafe { OleInitialize(None)? };
        Ok(Self)
    }
}

impl Drop for Apartment {
    fn drop(&mut self) {
        // SAFETY: exactly one balance for the successful initialization above.
        unsafe { OleUninitialize() };
    }
}

fn native_error(error: windows_core::Error) -> DragError {
    DragError::Native(error.to_string())
}

pub(super) fn drag(bytes: Vec<u8>, cancelled: Arc<AtomicBool>) -> Result<DragOutcome, DragError> {
    // SAFETY: GetAsyncKeyState takes a numeric virtual-key constant; it has no
    // pointer/lifetime contract. Do not start a late drag after mouse release.
    if cancelled.load(Ordering::Acquire)
        || unsafe { GetAsyncKeyState(VK_LBUTTON.0 as i32) } >= 0
        || unsafe { GetAsyncKeyState(VK_ESCAPE.0 as i32) } < 0
    {
        return Ok(DragOutcome::Cancelled);
    }
    let _apartment = Apartment::enter().map_err(native_error)?;
    let object = data_object_from_bytes(bytes)?;
    release_pointer_capture()?;
    let source: IDropSource = CopySource { cancelled }.into();
    let mut effect = DROPEFFECT_NONE;
    // SAFETY: both reference-counted objects and effect remain alive throughout
    // the modal call on this initialized STA. Only copying is permitted.
    let status = unsafe { DoDragDrop(&object, &source, DROPEFFECT_COPY, &mut effect) };
    outcome(status, effect)
}

fn outcome(status: HRESULT, effect: DROPEFFECT) -> Result<DragOutcome, DragError> {
    if status == DRAGDROP_S_CANCEL {
        return Ok(DragOutcome::Cancelled);
    }
    status.ok().map_err(native_error)?;
    if status == DRAGDROP_S_DROP && effect.0 & DROPEFFECT_COPY.0 != 0 {
        Ok(DragOutcome::Accepted)
    } else {
        Ok(DragOutcome::Rejected)
    }
}

pub(super) fn drop_bytes(sources: &Sources) -> Result<Vec<u8>, DragError> {
    // DROPFILES has a stable 20-byte Windows ABI, followed by UTF-16 paths.
    const { assert!(size_of::<DROPFILES>() == 20) };
    let mut bytes = Vec::from(20_u32.to_le_bytes());
    bytes.extend_from_slice(&[0; 12]); // POINT and fNC
    bytes.extend_from_slice(&1_i32.to_le_bytes()); // fWide
    for path in &sources.0 {
        let units = shell_path(path)?;
        if units.is_empty()
            || units.len() > MAX_PATH_UNITS
            || units.contains(&0)
            || bytes.len() + (units.len() + 2) * 2 > MAX_PAYLOAD_BYTES
        {
            return Err(DragError::InvalidPath);
        }
        for unit in units {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
        bytes.extend_from_slice(&0_u16.to_le_bytes());
    }
    bytes.extend_from_slice(&0_u16.to_le_bytes());
    Ok(bytes)
}

fn shell_path(path: &Path) -> Result<Vec<u16>, DragError> {
    let units: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .take(MAX_PATH_UNITS + 1)
        .collect();
    if units.is_empty() || units.len() > MAX_PATH_UNITS || units.contains(&0) {
        return Err(DragError::InvalidPath);
    }
    for component in path.components() {
        match component {
            Component::ParentDir | Component::CurDir => return Err(DragError::ShellPath),
            Component::Normal(name) => {
                let name: Vec<u16> = name.encode_wide().collect();
                if reserved_name(&name)
                    || name
                        .last()
                        .is_some_and(|unit| *unit == u16::from(b' ') || *unit == u16::from(b'.'))
                    || name
                        .iter()
                        .any(|unit| b"\"*:<>?|".iter().any(|byte| u16::from(*byte) == *unit))
                {
                    return Err(DragError::ShellPath);
                }
            }
            _ => {}
        }
    }
    // Catalog locators use canonical extended paths. Shell accepted their drops
    // but copied nothing in our native test. Remove only drive/UNC prefixes after
    // rejecting components whose meaning would change without verbatim semantics.
    match path.components().next() {
        Some(Component::Prefix(prefix)) => match prefix.kind() {
            Prefix::VerbatimDisk(_) => Ok(units[4..].to_vec()), // strip \\?\
            Prefix::VerbatimUNC(_, _) => {
                let mut unc: Vec<u16> = "\\\\".encode_utf16().collect();
                unc.extend_from_slice(&units[8..]); // replace \\?\UNC\ with \\
                Ok(unc)
            }
            Prefix::Disk(_) | Prefix::UNC(_, _) => Ok(units),
            Prefix::Verbatim(_) | Prefix::DeviceNS(_) => Err(DragError::ShellPath),
        },
        _ => Err(DragError::ShellPath),
    }
}

fn reserved_name(name: &[u16]) -> bool {
    let end = name
        .iter()
        .position(|unit| *unit == u16::from(b'.'))
        .unwrap_or(name.len());
    // This string is used only to detect reserved names, never as a file locator.
    // A stem with unpaired surrogates cannot equal any of these reserved names.
    let Ok(stem) = String::from_utf16(&name[..end]) else {
        return false;
    };
    let stem = stem.trim_end_matches(' ').to_ascii_uppercase();
    matches!(
        stem.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) || stem
        .strip_prefix("COM")
        .or_else(|| stem.strip_prefix("LPT"))
        .is_some_and(|suffix| {
            matches!(
                suffix,
                "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
            )
        })
}

fn data_object_from_bytes(bytes: Vec<u8>) -> Result<IDataObject, DragError> {
    // SAFETY: a static, null-terminated Windows clipboard format name.
    let preferred = unsafe { RegisterClipboardFormatW(CFSTR_PREFERREDDROPEFFECT) };
    if preferred == 0 || preferred > u16::MAX as u32 {
        return Err(native_error(windows_core::Error::from_thread()));
    }
    Ok(FileOffer {
        bytes,
        preferred: preferred as u16,
    }
    .into())
}

fn format(id: u16) -> FORMATETC {
    FORMATETC {
        cfFormat: id,
        dwAspect: DVASPECT_CONTENT.0,
        lindex: -1,
        tymed: TYMED_HGLOBAL.0 as u32,
        ..Default::default()
    }
}

struct GlobalMemory(HGLOBAL);

impl GlobalMemory {
    fn copy(bytes: &[u8]) -> windows_core::Result<Self> {
        // SAFETY: allocation size equals the slice length; this guard owns the
        // handle until ownership is transferred into STGMEDIUM.
        let memory = Self(unsafe { GlobalAlloc(GMEM_MOVEABLE, bytes.len())? });
        let pointer = unsafe { GlobalLock(memory.0) };
        if pointer.is_null() {
            return Err(windows_core::Error::from_thread());
        }
        // SAFETY: GlobalLock provides writable allocation of bytes.len() bytes,
        // and the slice is a separate live Rust allocation. No pointers escape.
        unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), pointer.cast(), bytes.len()) };
        // GlobalUnlock returns FALSE on a successful final unlock; GetLastError
        // distinguishes that documented case from an actual failure.
        unsafe { SetLastError(ERROR_SUCCESS) };
        let unlocked = unsafe { GlobalUnlock(memory.0) };
        if let Err(error) = unlocked
            && unsafe { GetLastError() } != ERROR_SUCCESS
        {
            return Err(error);
        }
        Ok(memory)
    }

    fn into_medium(self) -> STGMEDIUM {
        let memory = ManuallyDrop::new(self);
        STGMEDIUM {
            tymed: TYMED_HGLOBAL.0 as u32,
            u: STGMEDIUM_0 { hGlobal: memory.0 },
            pUnkForRelease: ManuallyDrop::new(None),
        }
    }
}

impl Drop for GlobalMemory {
    fn drop(&mut self) {
        // SAFETY: only unpublished, uniquely owned allocations use this guard.
        if let Err(error) = unsafe { GlobalFree(Some(self.0)) } {
            eprintln!("File transfer memory cleanup failed: {error}");
        }
    }
}

#[implement(IDataObject)]
struct FileOffer {
    bytes: Vec<u8>,
    preferred: u16,
}

impl FileOffer_Impl {
    fn check(&self, requested: *const FORMATETC) -> windows_core::Result<FORMATETC> {
        if requested.is_null() {
            return Err(E_INVALIDARG.into());
        }
        // SAFETY: COM callers supply a readable FORMATETC for the call duration;
        // copy it immediately and never retain the caller's pointer or ptd.
        let requested = unsafe { *requested };
        if requested.cfFormat != CF_HDROP.0 && requested.cfFormat != self.preferred {
            return Err(DV_E_FORMATETC.into());
        }
        if requested.dwAspect != DVASPECT_CONTENT.0 {
            return Err(DV_E_DVASPECT.into());
        }
        if requested.lindex != -1 {
            return Err(DV_E_LINDEX.into());
        }
        if requested.tymed & TYMED_HGLOBAL.0 as u32 == 0 {
            return Err(DV_E_TYMED.into());
        }
        Ok(requested)
    }
}

impl IDataObject_Impl for FileOffer_Impl {
    fn GetData(&self, requested: *const FORMATETC) -> windows_core::Result<STGMEDIUM> {
        let requested = self.check(requested)?;
        let copy = DROPEFFECT_COPY.0.to_le_bytes();
        let bytes = if requested.cfFormat == self.preferred {
            &copy[..]
        } else {
            &self.bytes
        };
        // A fresh allocation for every request. The receiver calls ReleaseStgMedium.
        Ok(GlobalMemory::copy(bytes)?.into_medium())
    }

    fn QueryGetData(&self, requested: *const FORMATETC) -> HRESULT {
        self.check(requested)
            .map_or_else(|error| error.code(), |_| S_OK)
    }

    fn EnumFormatEtc(&self, direction: u32) -> windows_core::Result<IEnumFORMATETC> {
        if direction != DATADIR_GET.0 as u32 {
            return Err(E_NOTIMPL.into());
        }
        // SAFETY: Shell copies this two-element slice; ptd is null in both entries.
        unsafe { SHCreateStdEnumFmtEtc(&[format(CF_HDROP.0), format(self.preferred)]) }
    }

    fn GetCanonicalFormatEtc(&self, input: *const FORMATETC, output: *mut FORMATETC) -> HRESULT {
        let requested = match self.check(input) {
            Ok(requested) => requested,
            Err(error) => return error.code(),
        };
        if output.is_null() {
            return E_INVALIDARG;
        }
        // SAFETY: COM supplies a writable output FORMATETC; it does not own ptd.
        unsafe { output.write(format(requested.cfFormat)) };
        DATA_S_SAMEFORMATETC
    }

    fn GetDataHere(&self, _: *const FORMATETC, _: *mut STGMEDIUM) -> windows_core::Result<()> {
        Err(E_NOTIMPL.into())
    }

    fn SetData(
        &self,
        _: *const FORMATETC,
        _: *const STGMEDIUM,
        _: BOOL,
    ) -> windows_core::Result<()> {
        // Read-only offer: targets retain ownership of rejected SetData media.
        Err(E_NOTIMPL.into())
    }

    fn DAdvise(
        &self,
        _: *const FORMATETC,
        _: u32,
        _: Ref<IAdviseSink>,
    ) -> windows_core::Result<u32> {
        Err(OLE_E_ADVISENOTSUPPORTED.into())
    }

    fn DUnadvise(&self, _: u32) -> windows_core::Result<()> {
        Err(OLE_E_ADVISENOTSUPPORTED.into())
    }

    fn EnumDAdvise(&self) -> windows_core::Result<IEnumSTATDATA> {
        Err(OLE_E_ADVISENOTSUPPORTED.into())
    }
}

#[implement(IDropSource)]
struct CopySource {
    cancelled: Arc<AtomicBool>,
}

impl IDropSource_Impl for CopySource_Impl {
    fn QueryContinueDrag(&self, escape: BOOL, keys: MODIFIERKEYS_FLAGS) -> HRESULT {
        if escape.as_bool() || self.cancelled.load(Ordering::Acquire) {
            DRAGDROP_S_CANCEL
        } else if keys.0 & MK_LBUTTON.0 == 0 {
            let mut point = POINT::default();
            // SAFETY: point is writable for the call; WindowFromPoint inspects
            // the visible window under the release without changing any input.
            if let Err(error) = unsafe { GetCursorPos(&mut point) } {
                return error.code();
            }
            release_over_window(unsafe { WindowFromPoint(point) })
        } else {
            S_OK
        }
    }

    fn GiveFeedback(&self, _: DROPEFFECT) -> HRESULT {
        DRAGDROP_S_USEDEFAULTCURSORS
    }
}

fn release_over_window(window: HWND) -> HRESULT {
    let mut process = 0;
    // SAFETY: Win32 validates the window handle and writes only this u32 output.
    // Invalid/no windows have no drop receiver. Cancellation in our own process
    // prevents winit's incoming file handler from accepting a self-drop.
    let thread = unsafe { GetWindowThreadProcessId(window, Some(&mut process)) };
    if thread == 0 || process == unsafe { GetCurrentProcessId() } {
        DRAGDROP_S_CANCEL
    } else {
        DRAGDROP_S_DROP
    }
}

#[cfg(all(test, sift_local_tests))]
mod tests {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/unit/platform/windows_drag.rs"
    ));
}
