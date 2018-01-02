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
mod test {
    use std;
    use libc;

    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    use tempdir::TempDir;

    pub enum FsItem<'a> {
        Dir(&'a [u8], &'a [FsItem<'a>]),
        File(&'a [u8], &'a str),
        DirLink(&'a [u8], &'a [u8], &'a [u8]),
        FileLink(&'a [u8], &'a [u8], &'a str),
        DeadLink(&'a [u8], &'a [u8]),
        Empty,
    }

    pub fn create_fsitem(dir_fd: &::fd::Fd, item: &FsItem) {
        use std::io::Write;

        match item {
            &FsItem::Dir(name, content) => {
                let path = OsString::from_vec(name.to_vec());

                if name != b"." {
                    dir_fd
                        .mkdirat(&path, 0o777)
                        .expect(&format!("failed to create dir {:?}", name));
                }

                let sub_fd = dir_fd.openat(&path, 0
                                           | libc::O_RDONLY | libc::O_DIRECTORY
                                           | libc::O_CLOEXEC)
                    .expect(&format!("failed to open dir {:?}", path));

                for i in content {
                    create_fsitem(&sub_fd, i);
                }
            }

            &FsItem::File(name, content) => {
                let path = OsString::from_vec(name.to_vec());
                let fs_fd =
                    dir_fd.createat(&path, 0
                                    | libc::O_WRONLY | libc::O_CLOEXEC
                                    | libc::O_EXCL, 0o0666)
                    .expect(&format!("failed to create file {:?}", path));

                fs_fd.into_file()
                    .expect(&format!("failed to transform file {:?}", path))
                    .write_all(content.as_bytes())
                    .expect(&format!("failed to fill file {:?}", path));
            }

            &FsItem::DirLink(name, dst, _) |
            &FsItem::FileLink(name, dst, _) |
            &FsItem::DeadLink(name, dst) => {
                let path = OsString::from_vec(name.to_vec());
                let target = OsString::from_vec(dst.to_vec());

                dir_fd.symlinkat(&target, &path)
                    .expect(&format!("failed to create {:?} -> {:?} link",
                                     path, target));
            }

            &FsItem::Empty => { },
        }
    }

    pub fn create_fs(tmpdir: &std::path::Path, item: &FsItem) {
        let fd =
            ::fd::Fd::open(&tmpdir, 0
                           | libc::O_RDONLY | libc::O_DIRECTORY
                           | libc::O_CLOEXEC)
            .expect(&format!("failed to open tmpdir {:?}", tmpdir));

        create_fsitem(&fd, item);
    }

    pub fn create_tmpdir() -> TempDir {
        let res = TempDir::new("unix-fd-test")
            .expect("failed to create tmpdir");

        res
    }
}
