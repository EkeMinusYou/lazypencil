//! Private per-profile Unix endpoints, independent of long user-data paths.
use super::*;
use sha2::{Digest, Sha256};
use std::os::{
    fd::{AsRawFd, FromRawFd},
    unix::{
        ffi::OsStrExt,
        fs::{FileTypeExt, MetadataExt, PermissionsExt},
    },
};

pub(super) struct Endpoint {
    pub path: PathBuf,
}

impl Endpoint {
    pub fn new(profile: &Profile) -> Result<Self> {
        use std::os::unix::fs::DirBuilderExt;
        let temporary = fs::canonicalize("/tmp").map_err(|_| failure("profile_channel_root"))?;
        let root = temporary.join(format!("lp-pages-{}", unsafe { libc::geteuid() }));
        if let Err(error) = fs::DirBuilder::new().mode(0o700).create(&root)
            && error.kind() != io::ErrorKind::AlreadyExists
        {
            return Err(failure("profile_channel_root"));
        }
        super::super::private_metadata(&root, true)?;
        let key = crate::document::hash(
            &[
                profile.directory().as_os_str().as_bytes(),
                b"\0",
                profile.identity().as_bytes(),
            ]
            .concat(),
        );
        Ok(Self {
            path: root.join(format!("{key}.sock")),
        })
    }

    pub fn check(&self) -> io::Result<fs::Metadata> {
        let metadata = fs::symlink_metadata(&self.path)?;
        if !metadata.file_type().is_socket()
            || metadata.uid() != unsafe { libc::geteuid() }
            || metadata.permissions().mode() & 0o077 != 0
            || metadata.nlink() != 1
        {
            return Err(io::Error::from(io::ErrorKind::PermissionDenied));
        }
        Ok(metadata)
    }

    pub fn connect(&self, budget: &Budget<'_>) -> io::Result<UnixStream> {
        self.check()?;
        let raw = unsafe { libc::socket(libc::AF_UNIX, libc::SOCK_STREAM, 0) };
        if raw < 0 {
            return Err(io::Error::last_os_error());
        }
        let stream = unsafe { UnixStream::from_raw_fd(raw) };
        stream.set_nonblocking(true)?;
        if unsafe { libc::fcntl(raw, libc::F_SETFD, libc::FD_CLOEXEC) } < 0 {
            return Err(io::Error::last_os_error());
        }
        let mut address: libc::sockaddr_un = unsafe { std::mem::zeroed() };
        address.sun_family = libc::AF_UNIX as _;
        let name = self.path.as_os_str().as_bytes();
        if name.len() >= address.sun_path.len() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Profile socket path is too long",
            ));
        }
        for (destination, byte) in address.sun_path.iter_mut().zip(name) {
            *destination = *byte as _;
        }
        let length = std::mem::offset_of!(libc::sockaddr_un, sun_path) + name.len() + 1;
        #[cfg(target_os = "macos")]
        {
            address.sun_len = length as _;
        }
        let connected = unsafe {
            libc::connect(
                raw,
                (&address as *const libc::sockaddr_un).cast(),
                length as _,
            )
        };
        if connected != 0 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() != Some(libc::EINPROGRESS) {
                return Err(error);
            }
            loop {
                let remaining = budget
                    .remaining()
                    .map_err(|_| io::Error::from(io::ErrorKind::TimedOut))?;
                let mut descriptor = libc::pollfd {
                    fd: raw,
                    events: libc::POLLOUT,
                    revents: 0,
                };
                let ready = unsafe {
                    libc::poll(
                        &mut descriptor,
                        1,
                        remaining.as_millis().clamp(1, 50) as i32,
                    )
                };
                if ready < 0 {
                    let error = io::Error::last_os_error();
                    if error.kind() == io::ErrorKind::Interrupted {
                        continue;
                    }
                    return Err(error);
                }
                if ready == 0 {
                    continue;
                }
                let mut error: libc::c_int = 0;
                let mut size = std::mem::size_of_val(&error) as libc::socklen_t;
                if unsafe {
                    libc::getsockopt(
                        raw,
                        libc::SOL_SOCKET,
                        libc::SO_ERROR,
                        (&mut error as *mut libc::c_int).cast(),
                        &mut size,
                    )
                } != 0
                {
                    return Err(io::Error::last_os_error());
                }
                if error != 0 {
                    return Err(io::Error::from_raw_os_error(error));
                }
                break;
            }
        }
        stream.set_nonblocking(false)?;
        check_peer(&stream)?;
        Ok(stream)
    }
}

pub(super) fn check_peer(stream: &UnixStream) -> io::Result<()> {
    #[cfg(target_os = "macos")]
    {
        let mut user: libc::uid_t = 0;
        let mut group: libc::gid_t = 0;
        if unsafe { libc::getpeereid(stream.as_raw_fd(), &mut user, &mut group) } != 0 {
            return Err(io::Error::last_os_error());
        }
        if user != unsafe { libc::geteuid() } {
            return Err(io::Error::from(io::ErrorKind::PermissionDenied));
        }
    }
    #[cfg(target_os = "linux")]
    {
        let mut credentials: libc::ucred = unsafe { std::mem::zeroed() };
        let mut size = std::mem::size_of_val(&credentials) as libc::socklen_t;
        if unsafe {
            libc::getsockopt(
                stream.as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_PEERCRED,
                (&mut credentials as *mut libc::ucred).cast(),
                &mut size,
            )
        } != 0
        {
            return Err(io::Error::last_os_error());
        }
        if credentials.uid != unsafe { libc::geteuid() } {
            return Err(io::Error::from(io::ErrorKind::PermissionDenied));
        }
    }
    Ok(())
}

pub(super) fn context(
    profile: &Profile,
    browser: &Path,
    executable: &Path,
    budget: &Budget<'_>,
) -> Result<String> {
    use std::os::unix::fs::OpenOptionsExt;
    let mut file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
        .open(executable)
        .map_err(|_| failure("profile_engine"))?;
    let metadata = file.metadata().map_err(|_| failure("profile_engine"))?;
    if !metadata.is_file() || metadata.len() > 1024 * 1024 * 1024 {
        return Err(failure("profile_engine"));
    }
    let mut engine = Sha256::new();
    let mut buffer = vec![0; 64 * 1024];
    let mut length = 0u64;
    loop {
        budget.check()?;
        let count = file
            .read(&mut buffer)
            .map_err(|_| failure("profile_engine"))?;
        if count == 0 {
            break;
        }
        length = length.saturating_add(count as u64);
        if length > metadata.len() {
            return Err(failure("profile_engine"));
        }
        engine.update(&buffer[..count]);
    }
    if length != metadata.len() {
        return Err(failure("profile_engine"));
    }
    let mut context = Sha256::new();
    context.update(b"lazypencil-profile-page-v1");
    for value in [
        profile.directory().as_os_str().as_bytes(),
        profile.identity().as_bytes(),
        browser.as_os_str().as_bytes(),
    ] {
        context.update((value.len() as u64).to_be_bytes());
        context.update(value);
    }
    context.update(engine.finalize());
    Ok(context
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}
