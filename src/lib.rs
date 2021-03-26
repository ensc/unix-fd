#![allow(clippy::redundant_field_names)]

#[macro_use]
extern crate log;

#[macro_use]
extern crate error_chain;

use std::ffi::CString;
use std::path::Path;

pub mod errors {
    error_chain! {
        foreign_links {
            Io(::std::io::Error) #[cfg(unix)];
        }
    }
}

pub(crate) trait LibcString {
    fn as_libc(&self) -> (*const i8, CString);
}

impl LibcString for Path {
    fn as_libc(&self) -> (*const i8, CString) {
        let res = unsafe {
            use std::os::unix::ffi::OsStrExt;

            CString::from_vec_unchecked(self.as_os_str().as_bytes().to_vec())
        };

        (res.as_ptr() as *const i8, res)
    }
}


pub mod fd;
pub mod dir;
pub mod chroot;

#[cfg(test)]
extern crate libc;
#[cfg(test)]
extern crate tempdir;
#[cfg(test)]
extern crate env_logger;

#[cfg(test)]
#[path="tests/core.inc.rs"]
mod test;
