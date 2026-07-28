use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::Path;

#[cfg(windows)]
mod windows;

/// Options for opening a file whose contents must be restricted to the
/// current operating-system user.
#[derive(Debug, Default)]
pub struct PrivateFileOptions {
    read: bool,
    write: bool,
    truncate: bool,
    create: bool,
    create_new: bool,
}

impl PrivateFileOptions {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn read(&mut self, read: bool) -> &mut Self {
        self.read = read;
        self
    }

    pub fn write(&mut self, write: bool) -> &mut Self {
        self.write = write;
        self
    }

    pub fn truncate(&mut self, truncate: bool) -> &mut Self {
        self.truncate = truncate;
        self
    }

    pub fn create(&mut self, create: bool) -> &mut Self {
        self.create = create;
        self
    }

    pub fn create_new(&mut self, create_new: bool) -> &mut Self {
        self.create_new = create_new;
        self
    }

    pub fn open(&self, path: impl AsRef<Path>) -> io::Result<File> {
        let mut options = OpenOptions::new();
        options
            .read(self.read)
            .write(self.write)
            .truncate(self.truncate)
            .create(self.create)
            .create_new(self.create_new);

        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
        }
        #[cfg(windows)]
        {
            windows::configure_access(&mut options, self.read, self.write);
        }

        let file = options.open(path)?;
        protect(&file)?;
        Ok(file)
    }
}

/// Writes and flushes a private file, replacing any existing contents.
pub fn write_private(path: impl AsRef<Path>, data: &[u8]) -> io::Result<()> {
    let mut options = PrivateFileOptions::new();
    let mut file = options.write(true).create(true).truncate(true).open(path)?;
    file.write_all(data)?;
    file.sync_all()
}

/// Writes and flushes a new private file, returning `false` if it already
/// exists.
pub fn write_private_new(path: impl AsRef<Path>, data: &[u8]) -> io::Result<bool> {
    let mut options = PrivateFileOptions::new();
    let mut file = match options.write(true).create_new(true).open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => return Ok(false),
        Err(error) => return Err(error),
    };
    file.write_all(data)?;
    file.sync_all()?;
    Ok(true)
}

/// Creates or tightens a directory so only the current operating-system user
/// can access it. On Windows the owner-only ACE inherits to child files and
/// directories, including transient SQLite journal files.
pub fn ensure_private_dir(path: impl AsRef<Path>) -> io::Result<()> {
    let path = path.as_ref();
    match fs::symlink_metadata(path) {
        Ok(metadata) => validate_directory(path, &metadata)?,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                let mut builder = fs::DirBuilder::new();
                builder.recursive(true).mode(0o700);
                builder.create(path)?;
            }
            #[cfg(not(unix))]
            {
                fs::create_dir_all(path)?;
            }
            let metadata = fs::symlink_metadata(path)?;
            validate_directory(path, &metadata)?;
        }
        Err(error) => return Err(error),
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let metadata = fs::symlink_metadata(path)?;
        if metadata.uid() != unsafe { libc::geteuid() } {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "private directory is not owned by the current user",
            ));
        }
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))
    }
    #[cfg(windows)]
    {
        windows::protect_directory(path)
    }
    #[cfg(not(any(unix, windows)))]
    {
        Ok(())
    }
}

fn validate_directory(path: &Path, metadata: &fs::Metadata) -> io::Result<()> {
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "private directory path is not a real directory: {}",
                path.display()
            ),
        ));
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;
        if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "private directory path is a Windows reparse point: {}",
                    path.display()
                ),
            ));
        }
    }
    Ok(())
}

#[cfg(unix)]
fn protect(file: &File) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    file.set_permissions(std::fs::Permissions::from_mode(0o600))
}

#[cfg(windows)]
fn protect(file: &File) -> io::Result<()> {
    windows::protect(file)
}

#[cfg(not(any(unix, windows)))]
fn protect(_file: &File) -> io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn private_writes_are_owner_only_and_exclusive() {
        use std::os::unix::fs::PermissionsExt;

        let root = tempfile::tempdir().expect("temporary directory");
        let path = root.path().join("private");
        std::fs::write(&path, b"inherited mode").expect("seed ordinary file");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644))
            .expect("set inherited mode");

        write_private(&path, b"private").expect("replace privately");
        assert_eq!(std::fs::read(&path).expect("read private file"), b"private");
        assert_eq!(
            std::fs::metadata(&path)
                .expect("private metadata")
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        assert!(!write_private_new(&path, b"replacement").expect("exclusive private write"));
        assert_eq!(std::fs::read(&path).expect("read private file"), b"private");
    }

    #[cfg(unix)]
    #[test]
    fn private_open_refuses_a_symlink() {
        use std::os::unix::fs::symlink;

        let root = tempfile::tempdir().expect("temporary directory");
        let target = root.path().join("target");
        let link = root.path().join("link");
        std::fs::write(&target, b"target").expect("write target");
        symlink(&target, &link).expect("create symlink");

        let error = write_private(&link, b"private").expect_err("refuse symlink");
        assert_ne!(error.kind(), io::ErrorKind::NotFound);
        assert_eq!(std::fs::read(&target).expect("read target"), b"target");
    }

    #[cfg(unix)]
    #[test]
    fn private_directory_is_owner_only_and_refuses_a_symlink() {
        use std::os::unix::fs::{symlink, PermissionsExt};

        let root = tempfile::tempdir().expect("temporary directory");
        let directory = root.path().join("state/private");
        ensure_private_dir(&directory).expect("create private directory");
        assert_eq!(
            std::fs::metadata(&directory)
                .expect("private directory metadata")
                .permissions()
                .mode()
                & 0o777,
            0o700
        );

        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o755))
            .expect("loosen directory mode");
        ensure_private_dir(&directory).expect("tighten private directory");
        assert_eq!(
            std::fs::metadata(&directory)
                .expect("private directory metadata")
                .permissions()
                .mode()
                & 0o777,
            0o700
        );

        let link = root.path().join("private-link");
        symlink(&directory, &link).expect("create directory symlink");
        ensure_private_dir(&link).expect_err("refuse directory symlink");
    }

    #[cfg(windows)]
    #[test]
    fn private_writes_install_a_protected_current_user_only_dacl() {
        let root = tempfile::tempdir().expect("temporary directory");
        let path = root.path().join("private");
        std::fs::write(&path, b"inherited ACL").expect("seed ordinary file");

        write_private(&path, b"private").expect("protect existing file");
        assert_eq!(std::fs::read(&path).expect("read private file"), b"private");
        windows::assert_current_user_only_dacl(&path);
        assert!(!write_private_new(&path, b"replacement").expect("exclusive private write"));

        std::fs::remove_file(&path).expect("remove existing file");
        assert!(write_private_new(&path, b"new private").expect("create private file"));
        windows::assert_current_user_only_dacl(&path);
    }

    #[cfg(windows)]
    #[test]
    fn private_directory_dacl_is_protected_and_inherited_by_children() {
        let root = tempfile::tempdir().expect("temporary directory");
        let directory = root.path().join("state/private");

        ensure_private_dir(&directory).expect("create private directory");
        windows::assert_current_user_only_directory_dacl(&directory);

        let child = directory.join("index.sqlite3-wal");
        std::fs::write(&child, b"journal").expect("create inherited child");
        windows::assert_inherited_current_user_only_dacl(&child);
    }
}
