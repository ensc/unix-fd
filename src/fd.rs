extern crate errno;
extern crate libc;

use std;
use std::cell::Cell;
use std::io::Error;
use std::path::Path;
use std::ffi::OsString;
use std::os::unix::ffi::OsStringExt;

use std::mem;

use crate::errors::*;

use crate::LibcString;

macro_rules! try_errno {
    ($expr:expr) => {{
        let rc = $expr;

        ensure!(rc >= 0, Error::last_os_error());

        rc
    }}
}

#[allow(non_camel_case_types)]
type int = libc::c_int;

// wrap a file descriptor and close it automatically
#[derive(Debug)]
pub struct FdRaw {
    pub(crate) fd: libc::c_int,
    pub(crate) is_managed: Cell<bool>,
}

impl Drop for FdRaw {
    fn drop(&mut self) {
        if self.is_managed.get() {
            let rc = unsafe { libc::close(self.fd) };

            if rc < 0 {
                warn!("close({:?}) failed in drop(): {:?}", self,
                      Error::last_os_error());
            }
        }
    }
}

impl FdRaw {
    fn _new(fd: int) -> Self {
        Self {
            fd: fd,
            is_managed: Cell::new(fd >= 0 && fd != libc::AT_FDCWD),
        }
    }

    fn _new_unmanaged(fd: int) -> Self {
        Self {
            fd: fd,
            is_managed: Cell::new(false),
        }
    }

    pub fn into_file(self) -> Result<std::fs::File> {
        use std::os::unix::io::FromRawFd;

        let res = unsafe { std::fs::File::from_raw_fd(self.fd) };
        self.is_managed.set(false);

        Ok(res)
    }

    pub fn open<T: AsRef<Path>>(path: &T, flags: int) -> Result<Self> {
        let fd = try_errno!(unsafe {
            libc::open(path.as_ref().as_libc().0, flags)
        });

        Ok(Self::_new(fd))
    }

    pub fn openat<T: AsRef<Path>>(&self, path: &T, flags: int) -> Result<Self> {
        let fd = try_errno!(unsafe {
            libc::openat(self.fd, path.as_ref().as_libc().0, flags)
        });

        Ok(Self::_new(fd))
    }

    pub fn createat<T: AsRef<Path>>(&self, path: &T, flags: int,
                                    mode: u32) -> Result<Self>
    {
        let fd = try_errno!(unsafe {
            libc::openat(self.fd, path.as_ref().as_libc().0,
                         flags | libc::O_CREAT, mode)
        });

        Ok(Self::_new(fd))
    }

    pub fn mkdirat<T: AsRef<Path>>(&self, path: &T, mode: u32) -> Result<()> {
        try_errno!(unsafe {
            libc::mkdirat(self.fd, path.as_ref().as_libc().0, mode)
        });

        Ok(())
    }

    pub fn symlinkat<D,T>(&self, target: &D, path: &T) -> Result<()>
    where
        D: AsRef<Path>,
        T: AsRef<Path>,
    {
        try_errno!(unsafe {
            libc::symlinkat(target.as_ref().as_libc().0,
                            self.fd,
                            path.as_ref().as_libc().0)
        });

        Ok(())
    }

    pub unsafe fn new(fd: int) -> Self {
        assert!(fd >= 0);

        Self::_new(fd)
    }

    pub fn cwd() -> Self {
        Self::_new(libc::AT_FDCWD)
    }

    pub unsafe fn as_unmanaged(&self) -> Self {
        Self::_new_unmanaged(self.fd)
    }

    pub fn into_fd(self) -> Fd {
        Fd::from_rawfd(self)
    }

    pub fn dupfd(&self, cloexec: bool) -> Result<Self> {
        let cmd: int = match cloexec {
            true	=> libc::F_DUPFD_CLOEXEC,
            false	=> libc::F_DUPFD,
        };

        // start at fd 3 to avoid overriding some of the stdXXX
        let min_fd: int = 3;

        let fd = try_errno!(unsafe { libc::fcntl(self.fd, cmd, min_fd) });

        Ok(Self::_new(fd))
    }

    fn is_file_type(&self, fname: &Path, file_type: u32) -> bool {
        let stat = self.fstatat(&fname, false);
        match stat {
            Err(_) => false,
            Ok(s) => (s.st_mode & libc::S_IFMT) == file_type,
        }
    }

    pub fn is_lnkat<T: AsRef<Path>>(&self, fname: &T) -> bool {
        self.is_file_type(fname.as_ref(), libc::S_IFLNK)
    }

    pub fn is_regat<T: AsRef<Path>>(&self, fname: &T) -> bool {
        self.is_file_type(fname.as_ref(), libc::S_IFREG)
    }

    pub fn is_dirat<T: AsRef<Path>>(&self, fname: &T) -> bool {
        self.is_file_type(fname.as_ref(), libc::S_IFDIR)
    }

    pub fn stat<T>(fname: &T, do_follow: bool) -> Result<libc::stat>
    where
        T: AsRef<Path>
    {
	#[allow(clippy::uninit_assumed_init)]
        let mut stat: libc::stat = unsafe { mem::MaybeUninit::uninit().assume_init() };

        try_errno!(unsafe {
            if do_follow {
                libc::stat(fname.as_ref().as_libc().0, &mut stat)
            } else {
                libc::lstat(fname.as_ref().as_libc().0, &mut stat)
            }
        });

        Ok(stat)
    }

    pub fn fstatat<T>(&self, fname: &T, do_follow: bool) -> Result<libc::stat>
    where
        T: AsRef<Path>
    {
        let flags = if do_follow {
            0
        } else {
            libc::AT_SYMLINK_NOFOLLOW
        };

	#[allow(clippy::uninit_assumed_init)]
        let mut stat: libc::stat = unsafe { mem::MaybeUninit::uninit().assume_init() };

        try_errno!(unsafe {
            libc::fstatat(self.fd, fname.as_ref().as_libc().0, &mut stat, flags)
        });

        Ok(stat)
    }

    pub fn fstat(&self) -> Result<libc::stat> {
	#[allow(clippy::uninit_assumed_init)]
        let mut stat: libc::stat = unsafe { mem::MaybeUninit::uninit().assume_init() };

        try_errno!(unsafe {
            libc::fstat(self.fd, &mut stat)
        });

        Ok(stat)
    }

    pub fn readlinkat<T: AsRef<Path>>(&self, fname: &T) -> Result<OsString> {
        let mut buf = Vec::with_capacity(256);

        loop {
            let buf_sz = try_errno!(unsafe {
                // on overflow, readlinkat() returns buf.capacity();
                // else the number of actually written bytes
                libc::readlinkat(self.fd, fname.as_ref().as_libc().0,
                                 buf.as_mut_ptr() as *mut _,
                                 buf.capacity())
            }) as usize;

            assert!(buf_sz <= buf.capacity());

            unsafe {
                // set size; because readlinkat() returns <= capacity(),
                // this can be done directly.
                buf.set_len(buf_sz);
            }

            if buf_sz != buf.capacity() {
                return Ok(OsString::from_vec(buf));
            }

            // readlinkat() overflowed; reserve additional space and
            // try again...
            buf.reserve(256);
        }
    }
}

#[cfg(not(feature = "atomic-rc"))]
type Rc<T> = std::rc::Rc<T>;

#[cfg(feature = "atomic-rc")]
type Rc<T> = std::sync::Arc<T>;

#[derive(Clone, Debug)]
pub struct Fd(Rc<FdRaw>);

impl Fd {
    pub fn from_rawfd(fd: FdRaw) -> Self {
        Fd(Rc::new(fd))
    }

    pub fn to_fdraw(&self) -> &FdRaw {
        &self.0
    }

    pub fn open<T: AsRef<Path>>(path: &T, flags: int) -> Result<Self> {
        FdRaw::open(path, flags).map(Self::from_rawfd)
    }

    pub fn openat<T: AsRef<Path>>(&self, path: &T, flags: int) -> Result<Self> {
        self.0.openat(path, flags).map(Self::from_rawfd)
    }

    pub fn createat<T: AsRef<Path>>(&self, path: &T, flags:
                                    int, mode: u32) -> Result<Self> {
        self.0.createat(path, flags, mode).map(Self::from_rawfd)
    }

    pub fn cwd() -> Self {
        Self::from_rawfd(FdRaw::cwd())
    }

    pub fn into_rawfd(self) -> std::result::Result<FdRaw, Fd> {
        match Rc::try_unwrap(self.0) {
            Err(fd) => Err(Fd(fd)),
            Ok(fd) => Ok(fd),
        }
    }

    pub unsafe fn into_file(self) -> Result<std::fs::File>
    {
        self.into_rawfd().unwrap().into_file()
    }
}

impl std::ops::Deref for Fd {
    type Target = FdRaw;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

pub fn same_file_by_stat(a: &libc::stat, b: &libc::stat) -> bool {
    a.st_dev == b.st_dev && a.st_ino == b.st_ino && a.st_mode == b.st_mode
}
