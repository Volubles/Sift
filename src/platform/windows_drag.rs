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
        ERROR_SUCCESS, GetLastError, GlobalFree, HGLOBAL, LPARAM, OLE_E_ADVISENOTSUPPORTED, S_OK,
        SetLastError, WPARAM,
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
        Threading::GetCurrentThreadId,
    },
    UI::{
        Input::KeyboardAndMouse::{
            GetAsyncKeyState, GetCapture, ReleaseCapture, VK_ESCAPE, VK_LBUTTON,
        },
        Shell::{CFSTR_PREFERREDDROPEFFECT, DROPFILES, SHCreateStdEnumFmtEtc},
        WindowsAndMessaging::{MSG, PM_NOREMOVE, PeekMessageW, PostThreadMessageW, WM_KEYDOWN},
    },
};
use windows_core::{BOOL, HRESULT, Ref, implement};

use super::{DragError, DragOutcome, Sources};

const MAX_PATH_UNITS: usize = 32_767;
const MAX_PAYLOAD_BYTES: usize = 1024 * 1024;

pub(super) fn prepare_thread() -> u32 {
    // SAFETY: this worker owns its message queue. PeekMessage creates the queue
    // before its ID is published, so shutdown can wake a modal OLE loop.
    unsafe {
        let mut message = MSG::default();
        let _has_pending_message = PeekMessageW(&mut message, None, 0, 0, PM_NOREMOVE);
        GetCurrentThreadId()
    }
}

pub(super) fn wake_cancellation(thread: u32) {
    if thread == 0 {
        return;
    }
    // SAFETY: this posts only to our worker, never to the user's active window.
    // The shutdown flag is authoritative; the message just wakes OLE polling.
    if let Err(error) =
        unsafe { PostThreadMessageW(thread, WM_KEYDOWN, WPARAM(VK_ESCAPE.0 as usize), LPARAM(0)) }
    {
        // A worker that has already exited no longer needs cancellation.
        if error.code() != HRESULT::from_win32(1444) {
            eprintln!("File drag cancellation wake failed: {error}");
        }
    }
}

pub(super) fn release_pointer_capture() -> Result<(), DragError> {
    // SAFETY: called on the UI thread; GetCapture returns only that thread's
    // capture. OLE must acquire its own capture on the drag worker.
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
        // SAFETY: callers use a dedicated thread, and all COM objects are dropped
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

pub(super) fn drag(sources: &Sources, shutdown: Arc<AtomicBool>) -> Result<DragOutcome, DragError> {
    let _apartment = Apartment::enter().map_err(native_error)?;
    let object = data_object(sources)?;
    // SAFETY: GetAsyncKeyState takes a numeric virtual-key constant; it has no
    // pointer/lifetime contract. Do not start a late drag after mouse release.
    if shutdown.load(Ordering::Acquire) || unsafe { GetAsyncKeyState(VK_LBUTTON.0 as i32) } >= 0 {
        return Ok(DragOutcome::Cancelled);
    }
    let source: IDropSource = CopySource { shutdown }.into();
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

fn drop_bytes(sources: &Sources) -> Result<Vec<u8>, DragError> {
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

fn data_object(sources: &Sources) -> Result<IDataObject, DragError> {
    let bytes = drop_bytes(sources)?;
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
    shutdown: Arc<AtomicBool>,
}

impl IDropSource_Impl for CopySource_Impl {
    fn QueryContinueDrag(&self, escape: BOOL, keys: MODIFIERKEYS_FLAGS) -> HRESULT {
        if escape.as_bool() || self.shutdown.load(Ordering::Acquire) {
            DRAGDROP_S_CANCEL
        } else if keys.0 & MK_LBUTTON.0 == 0 {
            DRAGDROP_S_DROP
        } else {
            S_OK
        }
    }

    fn GiveFeedback(&self, _: DROPEFFECT) -> HRESULT {
        DRAGDROP_S_USEDEFAULTCURSORS
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;
    use std::path::PathBuf;
    use std::time::{Duration, Instant};
    use windows::Win32::{
        Foundation::POINTL,
        System::{
            Ole::{DROPEFFECT_MOVE, IDropTarget, ReleaseStgMedium},
            SystemServices::{MK_CONTROL, MK_SHIFT},
        },
        UI::{
            Shell::{
                BHID_SFUIObject, DragQueryFileW, HDROP, IShellItem, SHCreateItemFromParsingName,
            },
            WindowsAndMessaging::{DispatchMessageW, PM_REMOVE, TranslateMessage},
        },
    };
    use windows_core::PCWSTR;

    struct Medium(STGMEDIUM);

    impl Drop for Medium {
        fn drop(&mut self) {
            // SAFETY: each test medium owns exactly one successful GetData result.
            unsafe { ReleaseStgMedium(&mut self.0) };
        }
    }

    fn paths(medium: &Medium) -> Vec<Vec<u16>> {
        // SAFETY: all callers obtained this HGLOBAL medium from our IDataObject.
        unsafe {
            let drop = HDROP(medium.0.u.hGlobal.0);
            let count = DragQueryFileW(drop, u32::MAX, None);
            (0..count)
                .map(|index| {
                    let length = DragQueryFileW(drop, index, None) as usize;
                    let mut path = vec![0; length + 1];
                    assert_eq!(
                        DragQueryFileW(drop, index, Some(&mut path)) as usize,
                        length
                    );
                    path.truncate(length);
                    path
                })
                .collect()
        }
    }

    #[test]
    fn native_file_offer_preserves_wide_paths_and_owns_each_medium_independently() {
        let _apartment = Apartment::enter().unwrap();
        let wide = vec![67, 58, 92, 0xD800, 46, 119, 97, 118];
        let paths_in = vec![
            PathBuf::from("C:\\fixtures\\space å 🎵.wav"),
            PathBuf::from(OsString::from_wide(&wide)),
            PathBuf::from(format!("C:\\fixtures\\{}.wav", "a".repeat(400))),
        ];
        let sources = Sources::new(paths_in.clone()).unwrap();
        let object = data_object(&sources).unwrap();
        // SAFETY: each call receives a live FORMATETC; RAII releases both media.
        let first = Medium(unsafe { object.GetData(&format(CF_HDROP.0)) }.unwrap());
        let second = Medium(unsafe { object.GetData(&format(CF_HDROP.0)) }.unwrap());
        assert_ne!(unsafe { first.0.u.hGlobal }, unsafe { second.0.u.hGlobal });
        drop(first);
        assert_eq!(
            paths(&second),
            paths_in
                .iter()
                .map(|p| p.as_os_str().encode_wide().collect())
                .collect::<Vec<Vec<u16>>>()
        );
        let enumerator = unsafe { object.EnumFormatEtc(DATADIR_GET.0 as u32) }.unwrap();
        let mut formats = [FORMATETC::default(); 2];
        let mut fetched = 0;
        unsafe { enumerator.Next(&mut formats, Some(&mut fetched)) }
            .ok()
            .unwrap();
        assert_eq!(fetched, 2);
        assert_eq!(formats[0].cfFormat, CF_HDROP.0);
        let preferred = Medium(unsafe { object.GetData(&formats[1]) }.unwrap());
        let handle = unsafe { preferred.0.u.hGlobal };
        let pointer = unsafe { GlobalLock(handle) };
        assert!(!pointer.is_null());
        // SAFETY: the preferred effect has a four-byte DWORD representation.
        assert_eq!(
            unsafe { pointer.cast::<u32>().read_unaligned() },
            DROPEFFECT_COPY.0
        );
        unsafe { SetLastError(ERROR_SUCCESS) };
        let unlock = unsafe { GlobalUnlock(handle) };
        assert!(unlock.is_ok() || unsafe { GetLastError() } == ERROR_SUCCESS);
    }

    #[test]
    fn native_formats_and_payload_are_validated() {
        let _apartment = Apartment::enter().unwrap();
        let sources = Sources::new(vec![PathBuf::from("C:\\fixtures\\sound.wav")]).unwrap();
        let object = data_object(&sources).unwrap();
        assert_eq!(unsafe { object.QueryGetData(&format(CF_HDROP.0)) }, S_OK);
        assert_eq!(
            unsafe { object.QueryGetData(std::ptr::null()) },
            E_INVALIDARG
        );
        let mut requested = format(CF_HDROP.0);
        requested.lindex = 0;
        assert_eq!(unsafe { object.QueryGetData(&requested) }, DV_E_LINDEX);
        requested.lindex = -1;
        requested.tymed = 0;
        assert_eq!(unsafe { object.QueryGetData(&requested) }, DV_E_TYMED);
        requested.tymed = TYMED_HGLOBAL.0 as u32;
        requested.dwAspect = 0;
        assert_eq!(unsafe { object.QueryGetData(&requested) }, DV_E_DVASPECT);
        assert_eq!(unsafe { object.QueryGetData(&format(1)) }, DV_E_FORMATETC);
        let null_path = PathBuf::from(OsString::from_wide(&[67, 58, 92, 0, 97]));
        assert!(matches!(
            drop_bytes(&Sources(vec![null_path])),
            Err(DragError::InvalidPath)
        ));
        let long_path = PathBuf::from(format!("C:\\{}", "a".repeat(MAX_PATH_UNITS)));
        assert!(matches!(
            drop_bytes(&Sources(vec![long_path])),
            Err(DragError::InvalidPath)
        ));
        let large_path = PathBuf::from(format!("C:\\{}", "a".repeat(30_000)));
        assert!(matches!(
            drop_bytes(&Sources(vec![large_path; 20])),
            Err(DragError::InvalidPath)
        ));
    }

    #[test]
    fn shell_paths_convert_only_unambiguous_drive_and_unc_locators() {
        for (locator, offered) in [
            (r"\\?\C:\fixtures\å.wav", r"C:\fixtures\å.wav"),
            (r"\\?\UNC\server\share\å.wav", r"\\server\share\å.wav"),
        ] {
            assert_eq!(
                shell_path(Path::new(locator)).unwrap(),
                offered.encode_utf16().collect::<Vec<_>>()
            );
        }
        for locator in [
            r"\\?\C:\fixtures\trailing .wav.",
            r"\\?\C:\fixtures\trailing.wav ",
            r"\\?\C:\fixtures\con.wav",
            r"\\?\C:\fixtures\LPT¹.wav",
            r"\\?\C:\fixtures\..\sound.wav",
            r"\\?\C:\fixtures\sound.wav:stream",
            r"\\?\Volume{example}\sound.wav",
        ] {
            assert!(
                matches!(shell_path(Path::new(locator)), Err(DragError::ShellPath)),
                "{locator}"
            );
        }
    }

    #[test]
    fn copy_source_handles_escape_modifiers_and_shutdown_without_offering_move() {
        let _apartment = Apartment::enter().unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let source: IDropSource = CopySource {
            shutdown: stop.clone(),
        }
        .into();
        for keys in [MK_LBUTTON, MK_LBUTTON | MK_SHIFT, MK_LBUTTON | MK_CONTROL] {
            assert_eq!(unsafe { source.QueryContinueDrag(false, keys) }, S_OK);
        }
        assert_eq!(
            unsafe { source.QueryContinueDrag(false, MODIFIERKEYS_FLAGS(0)) },
            DRAGDROP_S_DROP
        );
        assert_eq!(
            unsafe { source.QueryContinueDrag(true, MK_LBUTTON) },
            DRAGDROP_S_CANCEL
        );
        stop.store(true, Ordering::Release);
        assert_eq!(
            unsafe { source.QueryContinueDrag(false, MK_LBUTTON) },
            DRAGDROP_S_CANCEL
        );
        assert_eq!(
            outcome(DRAGDROP_S_DROP, DROPEFFECT_MOVE).unwrap(),
            DragOutcome::Rejected
        );
        assert_eq!(
            outcome(DRAGDROP_S_DROP, DROPEFFECT_COPY).unwrap(),
            DragOutcome::Accepted
        );
        let object = data_object(&Sources(vec![PathBuf::from("C:\\fixtures\\sound.wav")])).unwrap();
        let mut effect = DROPEFFECT_NONE;
        // Wake the modal loop without sending keyboard input to another app.
        let thread = prepare_thread();
        wake_cancellation(thread);
        assert_eq!(
            unsafe { DoDragDrop(&object, &source, DROPEFFECT_COPY, &mut effect) },
            DRAGDROP_S_CANCEL
        );
    }

    #[test]
    #[ignore = "Exercises the actual Windows Shell folder drop target with disposable files"]
    fn shell_target_copies_multiple_unicode_sources_without_changing_originals() {
        let _apartment = Apartment::enter().unwrap();
        let directory = tempfile::tempdir().unwrap();
        let target_path = directory.path().join("destination");
        std::fs::create_dir(&target_path).unwrap();
        // A genuine, rights-cleared PCM WAV: 16 mono silence samples at 8 kHz.
        let mut wave = b"RIFF".to_vec();
        wave.extend_from_slice(&68_u32.to_le_bytes());
        wave.extend_from_slice(b"WAVEfmt ");
        wave.extend_from_slice(&16_u32.to_le_bytes());
        wave.extend_from_slice(&1_u16.to_le_bytes());
        wave.extend_from_slice(&1_u16.to_le_bytes());
        wave.extend_from_slice(&8000_u32.to_le_bytes());
        wave.extend_from_slice(&16000_u32.to_le_bytes());
        wave.extend_from_slice(&2_u16.to_le_bytes());
        wave.extend_from_slice(&16_u16.to_le_bytes());
        wave.extend_from_slice(b"data");
        wave.extend_from_slice(&32_u32.to_le_bytes());
        wave.extend_from_slice(&[0; 32]);
        let names = ["space å 🎵.wav", "second sound.wav", "long sound.wav"];
        let mut originals = Vec::new();
        for name in names {
            let parent = if name == "long sound.wav" {
                let mut parent = directory.path().to_owned();
                for index in 0..6 {
                    parent.push(format!("{index}_{}", "a".repeat(40)));
                }
                std::fs::create_dir_all(&parent).unwrap();
                assert!(parent.as_os_str().encode_wide().count() > 260);
                parent
            } else {
                directory.path().to_owned()
            };
            let path = parent.join(name);
            std::fs::write(&path, &wave).unwrap();
            // Match catalog locators, including the extended-length prefix.
            originals.push(path.canonicalize().unwrap());
        }
        let sources = Sources::new(originals.clone()).unwrap();
        assert!(sources.validate(&AtomicBool::new(false)).unwrap());
        let object = data_object(&sources).unwrap();
        let mut target_wide: Vec<u16> = target_path.as_os_str().encode_wide().collect();
        target_wide.push(0);
        // SAFETY: target_wide is null-terminated and lives through the call. Both
        // interfaces are local to this initialized STA and the folder is private.
        let target: IDropTarget = unsafe {
            let item: IShellItem =
                SHCreateItemFromParsingName(PCWSTR(target_wide.as_ptr()), None).unwrap();
            item.BindToHandler(None, &BHID_SFUIObject).unwrap()
        };
        let point = POINTL::default();
        let mut effect = DROPEFFECT_COPY;
        unsafe { target.DragEnter(&object, MK_LBUTTON, point, &mut effect) }.unwrap();
        assert_eq!(effect.0 & DROPEFFECT_COPY.0, DROPEFFECT_COPY.0);
        effect = DROPEFFECT_COPY;
        unsafe { target.Drop(&object, MODIFIERKEYS_FLAGS(0), point, &mut effect) }.unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if names
                .iter()
                .all(|name| std::fs::read(target_path.join(name)).is_ok_and(|bytes| bytes == wave))
            {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "Shell copy did not publish complete files: {:?}",
                std::fs::read_dir(&target_path)
                    .unwrap()
                    .map(|entry| entry.unwrap().file_name())
                    .collect::<Vec<_>>()
            );
            // Pump STA messages if Shell schedules work after accepting the drop.
            let mut message = MSG::default();
            while unsafe { PeekMessageW(&mut message, None, 0, 0, PM_REMOVE) }.as_bool() {
                let _translated = unsafe { TranslateMessage(&message) };
                unsafe { DispatchMessageW(&message) };
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        for original in originals {
            assert_eq!(std::fs::read(original).unwrap(), wave);
        }
    }
}
