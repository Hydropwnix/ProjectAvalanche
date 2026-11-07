use std::fs::{File, OpenOptions};
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
use std::os::windows::io::AsRawHandle;
use std::path::Path;

use windows_sys::Win32::Foundation::{ERROR_ACCESS_DENIED, ERROR_INVALID_PARAMETER};
use windows_sys::Win32::Storage::FileSystem::{
    FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
    FILE_ID_128, FILE_ID_INFO, FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, FileIdInfo,
    GetFileInformationByHandleEx, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct FileId(u64, [u8; 16]);

pub fn id(file: &File) -> io::Result<FileId> {
    let mut info = FILE_ID_INFO { VolumeSerialNumber: 0, FileId: FILE_ID_128 { Identifier: [0; 16] } };
    let ok = unsafe {
        GetFileInformationByHandleEx(
            file.as_raw_handle(),
            FileIdInfo,
            (&raw mut info).cast(),
            std::mem::size_of::<FILE_ID_INFO>() as u32,
        )
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(FileId(info.VolumeSerialNumber, info.FileId.Identifier))
}

pub const SHARE_ALL: u32 = FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE;
pub const NO_FOLLOW: u32 = FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS;

pub fn is_regular(file: &File) -> io::Result<bool> {
    let a = file.metadata()?.file_attributes();
    Ok(a & (FILE_ATTRIBUTE_DIRECTORY | FILE_ATTRIBUTE_REPARSE_POINT) == 0)
}

pub fn create_new(path: &Path, _mode: u32) -> io::Result<File> {
    OpenOptions::new().read(true).write(true).create_new(true).open(path)
}

pub fn open_regular(path: &Path) -> io::Result<Option<File>> {
    let file = OpenOptions::new().read(true).share_mode(SHARE_ALL).custom_flags(NO_FOLLOW).open(path)?;
    Ok(is_regular(&file)?.then_some(file))
}

pub fn id_of(path: &Path) -> io::Result<Option<FileId>> {
    let file = match OpenOptions::new()
        .access_mode(FILE_READ_ATTRIBUTES)
        .share_mode(SHARE_ALL)
        .custom_flags(NO_FOLLOW)
        .open(path)
    {
        Ok(f) => f,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    if !is_regular(&file)? {
        return Ok(None);
    }
    id(&file).map(Some)
}

pub fn mode_of(_path: &Path) -> Option<u32> {
    None
}

fn verbatim(path: &Path) -> io::Result<Vec<u16>> {
    let abs = std::path::absolute(path)?;
    let wide: Vec<u16> = abs.as_os_str().encode_wide().collect();
    let prefix = |s: &str| s.encode_utf16().collect::<Vec<u16>>();
    let mut out = if wide.starts_with(&prefix(r"\\?\")) || wide.starts_with(&prefix(r"\\.\")) {
        wide
    } else if let Some(unc) = wide.strip_prefix(prefix(r"\\").as_slice()) {
        [prefix(r"\\?\UNC\"), unc.to_vec()].concat()
    } else {
        [prefix(r"\\?\"), wide].concat()
    };
    if out.contains(&0) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "a NUL in the path"));
    }
    out.push(0);
    Ok(out)
}

fn move_file(from: &Path, to: &Path, flags: u32) -> io::Result<()> {
    let (f, t) = (verbatim(from)?, verbatim(to)?);
    if unsafe { MoveFileExW(f.as_ptr(), t.as_ptr(), flags) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

pub fn rename_replace(from: &Path, to: &Path) -> io::Result<()> {
    match move_file(from, to, MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH) {
        Err(e) if e.raw_os_error() == Some(ERROR_ACCESS_DENIED as i32) => match posix_replace(from, to) {
            Ok(()) => Ok(()),
            Err(p) if p.raw_os_error() == Some(ERROR_INVALID_PARAMETER as i32) => Err(e),
            Err(p) => Err(p),
        },
        r => r,
    }
}

// TODO(guess): the rename through a write-through handle is taken to be on the disk when it returns, as
// MOVEFILE_WRITE_THROUGH's is (ReactOS's MoveFileWithProgressW opens its source with FILE_WRITE_THROUGH and renames
// through that handle; Microsoft documents write-through for writes only); settle: Microsoft documentation of
// write-through for FileRenameInfoEx, or a power-cut test on NTFS.
fn posix_replace(from: &Path, to: &Path) -> io::Result<()> {
    use windows_sys::Win32::Storage::FileSystem::{
        DELETE, FILE_FLAG_WRITE_THROUGH, FILE_RENAME_INFO, FileRenameInfoEx, SYNCHRONIZE, SetFileInformationByHandle,
    };
    use windows_sys::Win32::System::WindowsProgramming::{
        FILE_RENAME_FLAG_POSIX_SEMANTICS, FILE_RENAME_FLAG_REPLACE_IF_EXISTS,
    };
    let file = OpenOptions::new()
        .access_mode(DELETE | SYNCHRONIZE)
        .share_mode(SHARE_ALL)
        .custom_flags(NO_FOLLOW | FILE_FLAG_WRITE_THROUGH)
        .open(from)?;
    let name = verbatim(to)?;
    let name = &name[..name.len() - 1];
    let name_bytes = u32::try_from(name.len() * 2)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "a path too long to rename"))?;
    let offset = std::mem::offset_of!(FILE_RENAME_INFO, FileName);
    let size = (offset + name.len() * 2 + 2).max(std::mem::size_of::<FILE_RENAME_INFO>());
    let mut buf = vec![0u64; size.div_ceil(8)];
    let info = buf.as_mut_ptr().cast::<FILE_RENAME_INFO>();
    unsafe {
        (&raw mut (*info).Anonymous.Flags).write(FILE_RENAME_FLAG_REPLACE_IF_EXISTS | FILE_RENAME_FLAG_POSIX_SEMANTICS);
        (&raw mut (*info).FileNameLength).write(name_bytes);
        std::ptr::copy_nonoverlapping(name.as_ptr(), (&raw mut (*info).FileName).cast::<u16>(), name.len());
    }
    let ok =
        unsafe { SetFileInformationByHandle(file.as_raw_handle(), FileRenameInfoEx, buf.as_ptr().cast(), size as u32) };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

pub fn rename_noreplace(from: &Path, to: &Path) -> io::Result<()> {
    move_file(from, to, MOVEFILE_WRITE_THROUGH)
}

// TODO(guess): with no folder sync, a name made in a folder (a file of an installed folder, a new folder of the
// setup) is taken as durable once a write-through rename after it returns (NTFS keeps one metadata log, written in
// order); settle: a power-cut test on NTFS, or Microsoft documentation of the order NTFS commits its log records in.
pub fn sync_folder(_dir: &Path) -> io::Result<()> {
    Ok(())
}

pub fn sync_file(path: &Path) -> io::Result<()> {
    OpenOptions::new()
        .write(true)
        .share_mode(SHARE_ALL)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)?
        .sync_all()
}

#[cfg(feature = "cap-std")]
pub fn handle_path(handle: &impl AsRawHandle) -> io::Result<std::path::PathBuf> {
    use std::os::windows::ffi::OsStringExt;
    use windows_sys::Win32::Storage::FileSystem::{FILE_NAME_NORMALIZED, GetFinalPathNameByHandleW, VOLUME_NAME_DOS};
    let mut buf = vec![0u16; 512];
    loop {
        let n = unsafe {
            GetFinalPathNameByHandleW(
                handle.as_raw_handle(),
                buf.as_mut_ptr(),
                buf.len() as u32,
                FILE_NAME_NORMALIZED | VOLUME_NAME_DOS,
            )
        } as usize;
        if n == 0 {
            return Err(io::Error::last_os_error());
        }
        if n < buf.len() {
            buf.truncate(n);
            return Ok(std::ffi::OsString::from_wide(&buf).into());
        }
        if n > 32_768 {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "a folder path longer than 32 767 characters"));
        }
        buf = vec![0u16; n];
    }
}
