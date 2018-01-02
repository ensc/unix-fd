extern crate libc;
extern crate errno;

use std::io::Error;
use std::fmt;
use std::ffi::{CStr, OsString, OsStr};
use std::os::unix::ffi::OsStrExt;

use fd::Fd;
use errors::*;

extern {
    // this is missing in libc crate :(
    pub fn fdopendir(fd: libc::c_int) -> *mut libc::DIR;
}

// wrap a DIR stream and destroy it automatically
#[derive(Debug)]
pub struct Dir {
    dirp: *mut libc::DIR,
}

impl Drop for Dir {
    fn drop(&mut self)
    {
        let rc = unsafe { libc::closedir(self.dirp) };

        if rc < 0 {
            warn!("closedir({:?}) failed in drop(): {:?}", self,
                  Error::last_os_error());
        }
    }
}

impl Dir {
    pub fn fdopendir(fd: &Fd) -> Result<Dir>
    {
        const FLAGS: libc::c_int = (libc::O_DIRECTORY | libc::O_CLOEXEC |
                                    libc::O_RDONLY | libc::O_NOFOLLOW);

        // do not use dupfd() here; fds share file offsets which is
        // usually not wanted
        let fd = fd.openat(&OsString::from("."), FLAGS)?;

        let dir = {
            let dir = unsafe { fdopendir(fd.fd) };
            ensure!(!dir.is_null(), Error::last_os_error());

            fd.is_managed.set(false);

            dir
        };

        Ok(Dir {
            dirp: dir,
        })
    }

    fn libc_readdir(&mut self) -> Result<*const libc::dirent>
    {
        errno::set_errno(errno::Errno(0));

        let entryp = unsafe { libc::readdir(self.dirp) };

        ensure!(!entryp.is_null() || errno::errno().0 == 0,
                Error::last_os_error());

        Ok(entryp)
    }

    pub fn readdir(self) -> ReadDir {
        ReadDir::new(self)
    }
}

#[derive(Copy, Clone)]
pub struct DirEntry {
    pub stat: libc::dirent,
}

impl DirEntry {
    pub fn name(&self) -> &OsStr {
        unsafe {
            let tmp = CStr::from_ptr(self.stat.d_name.as_ptr());
            OsStr::from_bytes(tmp.to_bytes())
        }
    }
}

impl fmt::Debug for DirEntry {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f,
               "dirent {{ ino={:?}, off={:?}, type={:?}, reclen={:?}, name='{:?}' }}",
               self.stat.d_ino, self.stat.d_off, self.stat.d_type,
               self.stat.d_reclen, self.name())
    }
}

pub struct ReadDir {
    dir: Dir,
    failed: bool,
}

impl ReadDir {
    pub fn new(dir: Dir) -> ReadDir {
        ReadDir {
            dir: dir,
            failed: false,
        }
    }
}

impl Iterator for ReadDir {
    type Item = Result<DirEntry>;

    fn next(&mut self) -> Option<Self::Item> {
        unsafe {
            loop {
                let entryp = self.dir.libc_readdir();
                let entry_raw = match entryp {
                    Err(e) => {
                        if self.failed {
                            return None;
                        } else {
                            self.failed = true;
                            return Some(Err(e).chain_err(|| "readdir() failed"));
                        }
                    }
                    Ok(e) => e,
                };

                if entry_raw.is_null() {
                    return None;
                }

                self.failed = false;

                let entry = DirEntry {
                    stat : *entry_raw
                };

                match entry.name().as_bytes() {
                    // ignore '.' and '..' directory entries
                    b"." | b".." => {}
                    _ => return Some(Ok(entry)),
                }
            }
        }
    }
}
