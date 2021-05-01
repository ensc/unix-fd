//! Userspace `chroot` implementation
extern crate libc;
extern crate error_chain;

use std::fmt;
use std::path::{Path, PathBuf};
use std::ffi::OsString;

use crate::fd::*;
use crate::dir::*;

use crate::errors::*;

const MAX_LOOP_CNT: u32 = 256;

struct ChdirLoopEnv {
    counter: u32,
    root_stat: Option<libc::stat>,
}

impl ChdirLoopEnv {
    fn new() -> ChdirLoopEnv {
        ChdirLoopEnv {
            counter: MAX_LOOP_CNT,
            root_stat: None,
        }
    }
}

impl fmt::Debug for ChdirLoopEnv {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "counter={:?}, root_stat={:?}",
               self.counter, self.root_stat.map(|_| "..."))
    }
}

struct DirInfo {
    is_root: bool,
    stat: libc::stat,
}

/// Userspace `chroot` environment
///
/// All symlinks below a root directory are resolved relative this
/// directory.  E.g. when having a directory tree like
///
/// ```text
/// /
/// |-- etc/
/// |   `-- passwd
/// `-- srv/
///     `-- www/
///         |-- etc/
///         |   `-- passwd
///         |-- tmp -> /etc/
///         |-- passwd -> /etc/passwd
///         `-- test -> ../../../etc/passwd
/// ```
///
/// All the `open()` statements in code like
///
/// ```
/// # extern crate libc;
/// # extern crate unix_fd;
/// #
/// # use std::ffi::OsString;
/// # use std::path::Path;
/// #
/// # type Chroot = unix_fd::chroot::Chroot;
/// #
/// let chroot = Chroot::new(&OsString::from("/srv/www"));
///
/// let fd = chroot.open(&Path::new("/etc/passwd"), libc::O_RDONLY);
/// let fd = chroot.open(&Path::new("/tmp/passwd"), libc::O_RDONLY);
/// let fd = chroot.open(&Path::new("/test"), libc::O_RDONLY);
/// let fd = chroot.open(&Path::new("/passwd"), libc::O_RDONLY);
/// ```
///
/// will access `/srv/www/etc/passwd` instead of `/etc/passwd`.
#[derive(Debug)]
pub struct Chroot {
    root: PathBuf
}

impl Chroot {
    pub fn new<T: AsRef<Path>>(root: &T) -> Self {
        Chroot {
            root: root.as_ref().to_path_buf(),
        }
    }

    /// Opens the top level directory of the chroot directory and
    /// returns the filedescriptor.
    ///
    /// The directory will be opened with `O_CLOEXEC` flag being set.
    pub fn root_fdraw(&self) -> Result<FdRaw> {
        let open_flags = libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_RDONLY;

        FdRaw::open(&self.root, open_flags)
    }

    pub fn root_fd(&self) -> Result<Fd> {
        let open_flags = libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_RDONLY;

        Fd::open(&self.root, open_flags)
    }

    fn dir_info(&self, dir_fd: &Fd, env: &mut ChdirLoopEnv) -> Result<DirInfo> {
        if env.root_stat.is_none() {
            env.root_stat = Some(Fd::cwd().fstatat(&self.root, true)?);
        }

        let root_stat = env.root_stat.as_ref().unwrap();

        let stat = dir_fd.fstatat(&".", false)?;
        let is_root =
            (stat.st_dev == root_stat.st_dev) &&
            (stat.st_ino == root_stat.st_ino);

        Ok(DirInfo {
            stat: stat,
            is_root: is_root,
        })
    }

    /// Opens the directory at `path` within the chroot.
    ///
    /// Every intermediate symlinks will be resolved relative to to
    /// the chroot.
    ///
    /// Restrictions: `path` must be absolute.
    pub fn chdir<T>(&self, path: &T) -> Result<Fd>
    where
        T: AsRef<Path>,
    {
        let path : &Path = path.as_ref();

        ensure!(path.is_absolute(), "path '{:?}' not absolute", path);

        let mut env: ChdirLoopEnv = ChdirLoopEnv::new();

        self.chdir_internal(Fd::cwd(), path, &mut env)
    }

    /// Opens a directory `path` in the chroot environment relative
    /// to `fd`.
    ///
    /// Behaviour is unspecified if `fd` lies outside the chroot.
    /// `path` can be relative.
    pub fn chdirat<T>(&self, dir_fd: &Fd, path:  &T) -> Result<Fd>
    where
        T: AsRef<Path>,
    {
        let mut env: ChdirLoopEnv = ChdirLoopEnv::new();

        self.chdir_internal(dir_fd.clone(), path.as_ref(), &mut env)
    }

    fn open_component(&self, dir_fd: Fd,
                      path: std::path::Component,
                      env: &mut ChdirLoopEnv) -> Result<Fd>
    {
	#[allow(clippy::identity_op)]
        let open_flags = 0
            | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_RDONLY
            | libc::O_NOFOLLOW;

        match path {
            std::path::Component::Prefix(_) => {
                unreachable!();
            },

            std::path::Component::ParentDir => {
                let info = self.dir_info(&dir_fd, env)?;

                if info.is_root {
                    Ok(dir_fd)
                } else {
                    dir_fd.openat(&"..", open_flags)
                }
            },

            std::path::Component::RootDir => {
                self.root_fd()
            },

            std::path::Component::CurDir => {
                Ok(dir_fd)
            },

            std::path::Component::Normal(p) => {
                dir_fd.openat(&p, open_flags)
            },
        }
    }

    fn chdir_internal(&self, dir_fd: Fd, path: &Path,
                      env: &mut ChdirLoopEnv) -> Result<Fd>
    {
        let mut dir_fd = dir_fd;

        for p in path.components() {
            use std::path::Component;

            dir_fd = match p {
                Component::Prefix(_) |
                Component::RootDir |
                Component::CurDir |
                Component::ParentDir =>
                    self.open_component(dir_fd, p, env)?,

                Component::Normal(path_name) => {
                    let tmp = Path::new(path_name);

                    if !dir_fd.is_lnkat(&tmp) {
                        self.open_component(dir_fd, p, env)?
                    } else if env.counter == 0 {
                        bail!("too much loops while resolving symbolic link '{:?}'",
                              path);
                    } else {
                        let new_path = dir_fd.readlinkat(&tmp)?;
                        let link = Path::new(&new_path);

                        env.counter -= 1;
                        let res = self.chdir_internal(dir_fd, link, env);
                        env.counter += 1;

                        res?
                    }
                }
            };
        }

        Ok(dir_fd)
    }

    fn opendir_internal(&self, dir_fd: &Fd, path: &Path, env: &mut ChdirLoopEnv)
                 -> Result<(Fd, OsString)>
    {
        let current_dir = OsString::from(".");
        let fdrc = dir_fd.clone();

        match path.parent() {
            None =>
                Ok((self.chdir_internal(fdrc, path, env)?, current_dir)),

            Some(p) => {
                Ok((
                    self.chdir_internal(fdrc, p, env)?,
                    path.file_name()
                        .unwrap_or_else(|| current_dir.as_os_str())
                        .to_os_string()))
            }
        }
    }

    /// Opens a file in the chroot relative to an open directory `fd`.
    ///
    /// Method first opens the directory containing `path` as described
    /// by `Self::chdirat()` and calls `openat()` with `O_NOFOLLOW
    /// being set there.
    pub fn openat<T>(&self, dir_fd: &Fd, path: &T, flags: libc::c_int)
                     -> Result<Fd>
    where
        T: AsRef<Path>,
    {

        let mut env = ChdirLoopEnv::new();
        let mut path = path.as_ref().to_owned();
        let mut num_loops = MAX_LOOP_CNT;

        while num_loops > 0 {
            let (dir_fd, comp) =
                self.opendir_internal(dir_fd, &path, &mut env)?;

            assert_eq!(env.counter, MAX_LOOP_CNT);

            if !dir_fd.is_lnkat(&comp) {
                return dir_fd.openat(&comp, flags | libc::O_NOFOLLOW);
            }

            path = Path::new(&dir_fd.readlinkat(&comp)?).to_owned();

            num_loops -= 1;
        }

        bail!("too much loops while resolving symbolic link '{:?}'",
              path);
    }

    /// Opens a file in the chroot environment.
    ///
    /// Method first opens the directory containing `path` as described
    /// by `Self::chdir()` and calls `openat()` with `O_NOFOLLOW being
    /// set there.
    pub fn open<T>(&self, path: &T, flags: libc::c_int)
                     -> Result<Fd>
    where
        T: AsRef<Path>,
    {
        self.openat(&self.root_fd()?, path, flags)
    }

    /// Checks whether path is a symlink
    ///
    /// Method returns when errors occurred while performing the
    /// lookup.
    pub fn is_lnkat<T>(&self, dir_fd: &Fd, path: &T) -> bool
    where
        T: AsRef<Path>,
    {
        let mut env = ChdirLoopEnv::new();

        self.opendir_internal(dir_fd, path.as_ref(), &mut env)
            .map(|(dir_fd, comp)| dir_fd.is_lnkat(&comp))
            .unwrap_or(false)
    }

    /// Checks whether path is a directory
    ///
    /// Method returns when errors occurred while performing the
    /// lookup.
    pub fn is_dirat<T>(&self, dir_fd: &Fd, path: &T) -> bool
    where
        T: AsRef<Path>,
    {
        let mut env = ChdirLoopEnv::new();

        self.opendir_internal(dir_fd, path.as_ref(), &mut env)
            .map(|(dir_fd, comp)| dir_fd.is_dirat(&comp))
            .unwrap_or(false)
    }

    /// Checks whether path is a regular file
    ///
    /// Method returns when errors occurred while performing the
    /// lookup.
    pub fn is_regat<T>(&self, dir_fd: &Fd, path: &T) -> bool
    where
        T: AsRef<Path>,
    {
        let mut env = ChdirLoopEnv::new();

        self.opendir_internal(dir_fd, path.as_ref(), &mut env)
            .map(|(dir_fd, comp)| dir_fd.is_regat(&comp))
            .unwrap_or(false)
    }

    /// Returns fstat information
    pub fn fstatat<T>(&self, dir_fd: &Fd, fname: &T) -> Result<libc::stat>
    where
        T: AsRef<Path>,
    {
        let do_follow = false;

        let mut env = ChdirLoopEnv::new();

        self.opendir_internal(dir_fd, fname.as_ref(), &mut env)
            .map(|(dir_fd, comp)| dir_fd.fstatat(&comp, do_follow))?
    }

    fn check_and_get_entry(dir_fd: &Fd, entry: &DirEntry,
                           info: &DirInfo) -> Result<Option<OsString>> {
        //const DT_UNKNOWN: u8 = libc::DT_UNKNOWN;
        const DT_UNKNOWN: u8 = 0;
        const DT_DIR: u8 = libc::DT_DIR;

        if entry.stat.d_ino != info.stat.st_ino {
            return Ok(None);
        }

        if entry.stat.d_type != DT_DIR && entry.stat.d_type != DT_UNKNOWN {
            return Ok(None);
        }

        let name = OsString::from(entry.name());
        let stat = dir_fd.fstatat(&name, false)?;

        if  ((stat.st_mode & libc::S_IFMT) != libc::S_IFDIR) ||
            stat.st_ino != info.stat.st_ino ||
            stat.st_dev != info.stat.st_dev {
            return Ok(None);
        }

        Ok(Some(name))
    }

    /// Transforms `fd` into an absolute path relative to the chroot
    /// and appends `fname` optionally.
    ///
    /// Note: this operation is expensive because it recurses into the
    /// parent directories of `fd` and iterates over their contents to
    /// look for a matching subdirectory.
    pub fn full_path<T>(&self, dir_fd: &Fd, fname: Option<&T>)
                        -> Result<OsString>
    where
        T: AsRef<Path>,
    {
        let parent_dir = Path::new("..");
        let mut res = Vec::new();
        let mut dir_fd = dir_fd.clone();
        let mut env: ChdirLoopEnv = ChdirLoopEnv::new();
        let mut total_size = 0;

        loop {
            let info = self.dir_info(&dir_fd, &mut env)?;

            assert_eq!(env.counter, MAX_LOOP_CNT);

            if info.is_root {
                break;
            }

            dir_fd = dir_fd.openat(&parent_dir,
                                   libc::O_CLOEXEC | libc::O_RDONLY |
                                   libc::O_DIRECTORY)?;

            let dir = Dir::fdopendir(&dir_fd)?;

            for e in ReadDir::new(dir) {
                let e_name = Self::check_and_get_entry(&dir_fd, &e?, &info)?;

                if let Some(name) = e_name {
                    total_size += name.len() + 1;
                    res.push(name);

                    break;
                }
            }

            if res.is_empty() {
                bail!("full_path(): no entry found");
            }
        }

        res.reverse();
        let mut path = OsString::with_capacity(total_size);

        if res.is_empty() && fname.is_none() {
            path.push("/");
        }

        for p in res {
            path.push("/");
            path.push(p);
        }

        match fname {
            None => {}
            Some(f) => {
                path.push("/");
                path.push(f.as_ref().as_os_str());
            }
        }

        Ok(path)
    }
}

#[cfg(test)]
#[path="tests/chroot-data.inc.rs"]
mod testdata;

#[cfg(test)]
#[path="tests/chroot.inc.rs"]
mod test;
