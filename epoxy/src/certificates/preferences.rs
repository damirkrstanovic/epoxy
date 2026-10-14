use super::{CertificateKey, SelectionError};
use serde::{Deserialize, Serialize};
use std::{
    ffi::CString,
    fs::{self, File},
    io::{self, Read, Write},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::{
            ffi::OsStrExt,
            fs::{MetadataExt, OpenOptionsExt},
        },
    },
    path::{Component, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};
static NEXT: AtomicU64 = AtomicU64::new(0);
const LIMIT: usize = 8192;
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Saved {
    version: u32,
    kind: String,
    key: CertificateKey,
}
#[derive(Clone)]
pub struct PreferenceStore {
    path: PathBuf,
}
impl PreferenceStore {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }
    fn directory(&self, create: bool) -> io::Result<(File, PathBuf)> {
        if !self.path.is_absolute() {
            return Err(io::Error::other("absolute path required"));
        }
        let parent = self
            .path
            .parent()
            .ok_or_else(|| io::Error::other("invalid parent"))?;
        let filename = self
            .path
            .file_name()
            .ok_or_else(|| io::Error::other("invalid filename"))?;
        let mut directory = File::open("/")?;
        for component in parent.components() {
            let Component::Normal(name) = component else {
                if matches!(component, Component::RootDir) {
                    continue;
                }
                return Err(io::Error::other("invalid path component"));
            };
            let name =
                CString::new(name.as_bytes()).map_err(|_| io::Error::other("invalid component"))?;
            if create {
                let code = unsafe { libc::mkdirat(directory.as_raw_fd(), name.as_ptr(), 0o700) };
                if code != 0 && io::Error::last_os_error().kind() != io::ErrorKind::AlreadyExists {
                    return Err(io::Error::last_os_error());
                }
            }
            let fd = unsafe {
                libc::openat(
                    directory.as_raw_fd(),
                    name.as_ptr(),
                    libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                )
            };
            if fd < 0 {
                return Err(io::Error::last_os_error());
            }
            directory = unsafe { File::from_raw_fd(fd) };
        }
        let metadata = directory.metadata()?;
        if metadata.uid() != unsafe { libc::geteuid() } || metadata.mode() & 0o7777 != 0o700 {
            return Err(io::Error::other("private directory required"));
        }
        let path = PathBuf::from(format!("/proc/self/fd/{}", directory.as_raw_fd())).join(filename);
        Ok((directory, path))
    }
    fn read(path: &PathBuf) -> io::Result<Option<(CertificateKey, fs::Metadata)>> {
        let mut file = match fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
            .open(path)
        {
            Ok(file) => file,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e),
        };
        let m = file.metadata()?;
        if !m.is_file()
            || m.uid() != unsafe { libc::geteuid() }
            || m.mode() & 0o7777 != 0o600
            || m.nlink() != 1
            || m.len() > LIMIT as u64
        {
            return Err(io::Error::other("invalid preference file"));
        }
        let mut bytes = Vec::new();
        (&mut file)
            .take((LIMIT + 1) as u64)
            .read_to_end(&mut bytes)?;
        if bytes.len() > LIMIT {
            return Err(io::Error::other("oversized preference"));
        }
        let saved: Saved =
            serde_json::from_slice(&bytes).map_err(|_| io::Error::other("invalid preference"))?;
        if saved.version != 1
            || saved.kind != "epoxy-certificate-selection"
            || saved.key.context.token_uri.is_empty()
            || saved.key.context.token_uri.contains(&0)
        {
            return Err(io::Error::other("unsupported preference"));
        }
        Ok(Some((saved.key, m)))
    }
    pub fn load(&self) -> Result<Option<CertificateKey>, SelectionError> {
        let (_directory, path) = match self.directory(false) {
            Ok(v) => v,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(_) => return Err(SelectionError::Preference),
        };
        Self::read(&path)
            .map(|v| v.map(|(key, _)| key))
            .map_err(|_| SelectionError::Preference)
    }
    pub fn save(&self, key: &CertificateKey) -> Result<(), SelectionError> {
        self.save_inner(key).map_err(|_| SelectionError::Preference)
    }
    fn save_inner(&self, key: &CertificateKey) -> io::Result<()> {
        let (directory, path) = self.directory(true)?;
        let previous = Self::read(&path)?.map(|(_, m)| m);
        let bytes = serde_json::to_vec(&Saved {
            version: 1,
            kind: "epoxy-certificate-selection".into(),
            key: key.clone(),
        })
        .map_err(|_| io::Error::other("invalid preference"))?;
        if bytes.len() > LIMIT {
            return Err(io::Error::other("oversized preference"));
        }
        let temporary = path.with_file_name(format!(
            ".selection-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&temporary)?;
        let result = (|| {
            file.write_all(&bytes)?;
            file.sync_all()?;
            Self::unchanged(&path, previous.as_ref())?;
            if previous.is_some() {
                fs::rename(&temporary, &path)?;
            } else {
                // Atomic no-replace for the first save; never clobber a newly created file.
                let old = CString::new(temporary.as_os_str().as_bytes()).unwrap();
                let new = CString::new(path.as_os_str().as_bytes()).unwrap();
                let code = unsafe {
                    libc::renameat2(
                        libc::AT_FDCWD,
                        old.as_ptr(),
                        libc::AT_FDCWD,
                        new.as_ptr(),
                        libc::RENAME_NOREPLACE,
                    )
                };
                if code != 0 {
                    return Err(io::Error::last_os_error());
                }
            }
            directory.sync_all()
        })();
        // The temporary name was created exclusively within the verified private directory.
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result
    }
    fn unchanged(path: &PathBuf, previous: Option<&fs::Metadata>) -> io::Result<()> {
        match (fs::symlink_metadata(path), previous) {
            (Err(e), None) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            (Ok(current), Some(old))
                if current.dev() == old.dev()
                    && current.ino() == old.ino()
                    && current.is_file()
                    && current.uid() == old.uid()
                    && current.mode() == old.mode()
                    && current.len() == old.len()
                    && current.mtime() == old.mtime()
                    && current.mtime_nsec() == old.mtime_nsec() =>
            {
                Ok(())
            }
            _ => Err(io::Error::other("preference changed")),
        }
    }
    pub fn forget(&self) -> Result<(), SelectionError> {
        let (directory, path) = match self.directory(false) {
            Ok(v) => v,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(_) => return Err(SelectionError::Preference),
        };
        let result = (|| {
            if let Some((_, m)) = Self::read(&path)? {
                Self::unchanged(&path, Some(&m))?;
                fs::remove_file(&path)?;
                directory.sync_all()?;
            }
            Ok::<(), io::Error>(())
        })();
        result.map_err(|_| SelectionError::Preference)
    }
}
