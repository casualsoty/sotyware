#![allow(non_snake_case)]

use std::ffi::{c_void, OsString};
use std::os::windows::ffi::OsStringExt;
use windows_sys::core::{HRESULT, PCWSTR, PWSTR};
pub use windows_sys::{
    core::GUID,
    Win32::{
        Foundation::HWND,
        UI::Shell::{Common::COMDLG_FILTERSPEC, FILEOPENDIALOGOPTIONS, SIGDN, SIGDN_FILESYSPATH},
    },
};

pub(crate) type Result<T> = std::result::Result<T, HRESULT>;

#[inline]
pub(super) fn wrap_err(hresult: HRESULT) -> Result<()> {
    if hresult >= 0 {
        Ok(())
    } else {
        Err(hresult)
    }
}

// sotyware patch: Windows file names are sequences of UTF-16 code units that
// need not be valid UTF-16 (an unpaired surrogate is allowed). They are kept
// losslessly as an `OsString`; upstream's `String::from_utf16(..).unwrap()`
// panicked on such a name, which aborts a `panic = "abort"` application.
#[inline]
unsafe fn read_to_os_string(ptr: *const u16) -> OsString {
    let mut cursor = ptr;

    while *cursor != 0 {
        cursor = cursor.add(1);
    }

    let slice = std::slice::from_raw_parts(ptr, cursor.offset_from(ptr) as usize);
    OsString::from_wide(slice)
}

#[repr(C)]
pub(super) struct Interface<T> {
    vtable: *mut T,
}

impl<T> Interface<T> {
    #[inline]
    pub(super) fn vtbl(&self) -> &T {
        unsafe { &*self.vtable }
    }
}

#[repr(C)]
pub(super) struct IUnknownV {
    __query_interface: usize,
    __add_ref: usize,
    pub(super) release: unsafe extern "system" fn(this: *mut c_void) -> u32,
}

pub(super) type IUnknown = Interface<IUnknownV>;

#[inline]
fn drop_impl(ptr: *mut c_void) {
    unsafe {
        ((*ptr.cast::<IUnknown>()).vtbl().release)(ptr);
    }
}

#[repr(C)]
pub(super) struct IShellItemV {
    base: IUnknownV,
    BindToHandler: unsafe extern "system" fn(
        this: *mut c_void,
        pbc: *mut c_void,
        bhid: *const GUID,
        riid: *const GUID,
        ppv: *mut *mut c_void,
    ) -> HRESULT,
    GetParent: unsafe extern "system" fn(this: *mut c_void, ppsi: *mut *mut c_void) -> HRESULT,
    GetDisplayName: unsafe extern "system" fn(
        this: *mut c_void,
        sigdnname: SIGDN,
        ppszname: *mut PWSTR,
    ) -> HRESULT,
    GetAttributes: usize,
    Compare: unsafe extern "system" fn(
        this: *mut c_void,
        psi: *mut c_void,
        hint: u32,
        piorder: *mut i32,
    ) -> HRESULT,
}

#[repr(transparent)]
pub(super) struct IShellItem(pub(super) *mut Interface<IShellItemV>);

impl IShellItem {
    pub(super) fn get_path(&self) -> Result<std::path::PathBuf> {
        let filename = unsafe {
            let mut dname = std::mem::MaybeUninit::uninit();
            wrap_err(((*self.0).vtbl().GetDisplayName)(
                self.0.cast(),
                SIGDN_FILESYSPATH,
                dname.as_mut_ptr(),
            ))?;

            let dname = dname.assume_init();
            let fname = read_to_os_string(dname);
            windows_sys::Win32::System::Com::CoTaskMemFree(dname.cast());
            fname
        };

        Ok(filename.into())
    }
}

impl Drop for IShellItem {
    fn drop(&mut self) {
        drop_impl(self.0.cast());
    }
}

#[repr(C)]
struct IShellItemArrayV {
    base: IUnknownV,
    BindToHandler: unsafe extern "system" fn(
        this: *mut c_void,
        pbc: *mut c_void,
        bhid: *const GUID,
        riid: *const GUID,
        ppvout: *mut *mut c_void,
    ) -> HRESULT,
    GetPropertyStore: usize,
    GetPropertyDescriptionList: usize,
    GetAttributes: usize,
    GetCount: unsafe extern "system" fn(this: *mut c_void, pdwnumitems: *mut u32) -> HRESULT,
    GetItemAt: unsafe extern "system" fn(
        this: *mut c_void,
        dwindex: u32,
        ppsi: *mut IShellItem,
    ) -> HRESULT,
    EnumItems:
        unsafe extern "system" fn(this: *mut c_void, ppenumshellitems: *mut *mut c_void) -> HRESULT,
}

#[repr(transparent)]
pub(super) struct IShellItemArray(*mut Interface<IShellItemArrayV>);

impl IShellItemArray {
    #[inline]
    pub(super) fn get_count(&self) -> Result<u32> {
        let mut count = 0;
        unsafe {
            wrap_err(((*self.0).vtbl().GetCount)(self.0.cast(), &mut count))?;
        }
        Ok(count)
    }

    #[inline]
    pub(super) fn get_item_at(&self, index: u32) -> Result<IShellItem> {
        let mut item = std::mem::MaybeUninit::uninit();
        unsafe {
            wrap_err(((*self.0).vtbl().GetItemAt)(
                self.0.cast(),
                index,
                item.as_mut_ptr(),
            ))?;
            Ok(item.assume_init())
        }
    }
}

impl Drop for IShellItemArray {
    fn drop(&mut self) {
        drop_impl(self.0.cast());
    }
}

#[repr(C)]
pub(super) struct IModalWindowV {
    base: IUnknownV,
    pub(super) Show: unsafe extern "system" fn(this: *mut c_void, owner: HWND) -> HRESULT,
}

/// <https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/nn-shobjidl_core-ifiledialog>
#[repr(C)]
pub(super) struct IFileDialogV {
    pub(super) base: IModalWindowV,
    pub(super) SetFileTypes: unsafe extern "system" fn(
        this: *mut c_void,
        cfiletypes: u32,
        rgfilterspec: *const COMDLG_FILTERSPEC,
    ) -> HRESULT,
    SetFileTypeIndex: unsafe extern "system" fn(this: *mut c_void, ifiletype: u32) -> HRESULT,
    GetFileTypeIndex: unsafe extern "system" fn(this: *mut c_void, pifiletype: *mut u32) -> HRESULT,
    Advise: unsafe extern "system" fn(
        this: *mut c_void,
        pfde: *mut c_void,
        pdwcookie: *mut u32,
    ) -> HRESULT,
    Unadvise: unsafe extern "system" fn(this: *mut c_void, dwcookie: u32) -> HRESULT,
    pub(super) SetOptions:
        unsafe extern "system" fn(this: *mut c_void, fos: FILEOPENDIALOGOPTIONS) -> HRESULT,
    GetOptions:
        unsafe extern "system" fn(this: *mut c_void, pfos: *mut FILEOPENDIALOGOPTIONS) -> HRESULT,
    SetDefaultFolder: unsafe extern "system" fn(this: *mut c_void, psi: *mut c_void) -> HRESULT,
    pub(super) SetFolder: unsafe extern "system" fn(this: *mut c_void, psi: *mut c_void) -> HRESULT,
    GetFolder: unsafe extern "system" fn(this: *mut c_void, ppsi: *mut *mut c_void) -> HRESULT,
    GetCurrentSelection:
        unsafe extern "system" fn(this: *mut c_void, ppsi: *mut *mut c_void) -> HRESULT,
    pub(super) SetFileName:
        unsafe extern "system" fn(this: *mut c_void, pszname: PCWSTR) -> HRESULT,
    GetFileName: unsafe extern "system" fn(this: *mut c_void, pszname: *mut PWSTR) -> HRESULT,
    pub(super) SetTitle: unsafe extern "system" fn(this: *mut c_void, psztitle: PCWSTR) -> HRESULT,
    SetOkButtonLabel: unsafe extern "system" fn(this: *mut c_void, psztext: PCWSTR) -> HRESULT,
    SetFileNameLabel: unsafe extern "system" fn(this: *mut c_void, pszlabel: PCWSTR) -> HRESULT,
    pub(super) GetResult:
        unsafe extern "system" fn(this: *mut c_void, ppsi: *mut IShellItem) -> HRESULT,
    AddPlace: unsafe extern "system" fn(
        this: *mut c_void,
        psi: *mut c_void,
        fdap: windows_sys::Win32::UI::Shell::FDAP,
    ) -> HRESULT,
    pub(super) SetDefaultExtension:
        unsafe extern "system" fn(this: *mut c_void, pszdefaultextension: PCWSTR) -> HRESULT,
    Close: unsafe extern "system" fn(this: *mut c_void, hr: HRESULT) -> HRESULT,
    SetClientGuid: unsafe extern "system" fn(this: *mut c_void, guid: *const GUID) -> HRESULT,
    ClearClientData: unsafe extern "system" fn(this: *mut c_void) -> HRESULT,
    SetFilter: unsafe extern "system" fn(this: *mut c_void, pfilter: *mut c_void) -> HRESULT,
}

#[repr(transparent)]
pub(super) struct IFileDialog(pub(super) *mut Interface<IFileDialogV>);

impl Drop for IFileDialog {
    fn drop(&mut self) {
        drop_impl(self.0.cast());
    }
}

/// <https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/nn-shobjidl_core-ifileopendialog>
#[repr(C)]
pub(super) struct IFileOpenDialogV {
    pub(super) base: IFileDialogV,
    /// <https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/nf-shobjidl_core-ifileopendialog-getresults>
    GetResults:
        unsafe extern "system" fn(this: *mut c_void, ppenum: *mut IShellItemArray) -> HRESULT,
    GetSelectedItems:
        unsafe extern "system" fn(this: *mut c_void, ppsai: *mut *mut c_void) -> HRESULT,
}

#[repr(transparent)]
pub(super) struct IFileOpenDialog(pub(super) *mut Interface<IFileOpenDialogV>);

impl IFileOpenDialog {
    #[inline]
    pub(super) fn get_results(&self) -> Result<IShellItemArray> {
        let mut res = std::mem::MaybeUninit::uninit();
        unsafe {
            wrap_err((((*self.0).vtbl()).GetResults)(
                self.0.cast(),
                res.as_mut_ptr(),
            ))?;
            Ok(res.assume_init())
        }
    }
}

impl Drop for IFileOpenDialog {
    fn drop(&mut self) {
        drop_impl(self.0.cast());
    }
}

// sotyware patch: regression tests for `read_to_os_string`, the conversion
// behind every path the Windows dialogs return (`get_result` for the
// open-file, folder and save dialogs, `get_results` for multi-selection).
#[cfg(test)]
mod tests {
    use super::{read_to_os_string, IShellItem};
    use crate::backend::win_cid::utils::init_com;
    use std::ffi::OsString;
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    use std::path::{Path, PathBuf};
    use windows_sys::{core::GUID, Win32::UI::Shell::SHCreateItemFromParsingName};

    const SHELL_ITEM_IID: GUID = GUID::from_u128(0x43826d1e_e718_42ee_bc55_a1e261c37bfe);

    fn nul_terminated(units: &[u16]) -> Vec<u16> {
        let mut v = units.to_vec();
        v.push(0);
        v
    }

    fn convert(units: &[u16]) -> OsString {
        let buf = nul_terminated(units);
        unsafe { read_to_os_string(buf.as_ptr()) }
    }

    fn units(path: &Path) -> Vec<u16> {
        path.as_os_str().encode_wide().collect()
    }

    #[test]
    fn ascii_path_is_read_unchanged() {
        let wide: Vec<u16> = r"C:\Users\test\report.txt".encode_utf16().collect();
        let path = PathBuf::from(convert(&wide));
        assert_eq!(path, Path::new(r"C:\Users\test\report.txt"));
        assert_eq!(units(&path), wide);
    }

    #[test]
    fn unicode_path_is_read_unchanged() {
        // Includes a character outside the BMP (a valid surrogate pair).
        let text = "C:\\Daten\\größe 日本 \u{1D11E}.bin";
        let wide: Vec<u16> = text.encode_utf16().collect();
        let path = PathBuf::from(convert(&wide));
        assert_eq!(path.to_str(), Some(text));
        assert_eq!(units(&path), wide);
    }

    #[test]
    fn empty_string_is_read_as_empty() {
        assert_eq!(convert(&[]), OsString::new());
    }

    #[test]
    fn unpaired_surrogates_are_kept_without_panic_or_loss() {
        let cases: [&[u16]; 4] = [
            &[0x43, 0x3A, 0x5C, 0x62, 0xD800, 0x2E, 0x62], // lone high
            &[0x43, 0x3A, 0x5C, 0x62, 0xDC00, 0x2E, 0x62], // lone low
            &[0x43, 0x3A, 0x5C, 0xDC00, 0xD800],           // reversed pair
            &[0x43, 0x3A, 0x5C, 0x61, 0xDBFF],             // high at the end
        ];
        for wide in cases {
            let path = PathBuf::from(convert(wide));
            // Lossless: the very same code units come back out.
            assert_eq!(units(&path), wide, "{wide:x?}");
            // Not valid Unicode, so not silently replaced by U+FFFD.
            assert_eq!(path.to_str(), None, "{wide:x?}");
            assert_eq!(path, PathBuf::from(OsString::from_wide(wide)));
        }
    }

    /// A unique, empty folder under the temporary folder, removed on drop.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(tag: &str) -> Self {
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or_default();
            let dir = std::env::temp_dir().join(format!(
                "rfd-sotyware-{tag}-{}-{nanos}",
                std::process::id()
            ));
            std::fs::create_dir(&dir).unwrap();
            Self(dir)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// What `IShellItem::get_path` (the dialogs' result conversion) returns
    /// for the shell item of an existing `path`.
    fn shell_item_path(path: &Path) -> PathBuf {
        let wide: Vec<u16> = nul_terminated(&units(path));
        init_com(|| unsafe {
            let mut item = std::mem::MaybeUninit::uninit();
            let hr = SHCreateItemFromParsingName(
                wide.as_ptr(),
                std::ptr::null_mut(),
                &SHELL_ITEM_IID,
                item.as_mut_ptr(),
            );
            assert!(hr >= 0, "SHCreateItemFromParsingName: 0x{hr:08X}");
            let item = IShellItem(item.assume_init().cast());
            item.get_path().unwrap()
        })
        .unwrap()
    }

    #[test]
    fn shell_items_return_the_original_file_system_path() {
        let dir = TempDir::new("item");
        let names = [
            OsString::from("plain.txt"),
            OsString::from("größe test \u{1D11E}.bin"),
            OsString::from("a + & # % b.txt"),
            // An unpaired surrogate: NTFS allows it, it is not valid Unicode.
            OsString::from_wide(&[0x6C, 0x2D, 0xD800, 0x2D, 0x73, 0x2E, 0x74]),
            OsString::from_wide(&[0x6C, 0x2D, 0xDC00, 0x2E, 0x74]),
        ];
        for name in names {
            let original = dir.0.join(&name);
            std::fs::write(&original, b"x").unwrap();
            let returned = shell_item_path(&original);
            assert_eq!(units(&returned), units(&original), "{name:?}");
            assert_eq!(returned.file_name(), Some(name.as_os_str()));
            // Usable as a file system path: it names the same file.
            assert_eq!(std::fs::read(&returned).unwrap(), b"x", "{name:?}");
        }
    }
}
