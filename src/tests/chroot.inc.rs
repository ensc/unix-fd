use super::testdata::*;

use std;
use std::io::Read;
use std::ffi::OsString;
use std::os::unix::ffi::OsStringExt;

use libc;

use test::FsItem;
use chroot::Chroot;

struct ChrootedChroot {
    dir: std::path::PathBuf,
    chroot: Chroot,
    root_fd: ::fd::Fd,
}

impl AsRef<Chroot> for ChrootedChroot {
    fn as_ref(&self) -> &Chroot {
        &self.chroot
    }
}

fn check_fsitem(root: &ChrootedChroot, dir_fd: &::fd::Fd, item: &FsItem) {

    let chroot = root.as_ref();

    match item {
        &FsItem::Empty => return,
        _ => {}
    }

    let (path, full_path) = match item {
        &FsItem::Dir(name, _) |
        &FsItem::File(name, _) |
        &FsItem::DirLink(name, _, _) |
        &FsItem::FileLink(name, _, _) |
        &FsItem::DeadLink(name, _) => {
            let path = OsString::from_vec(name.to_vec());

            let full_path = chroot.full_path(dir_fd, Some(&path)).expect(
                "failed to derive full path",
            );

            (path, full_path)
        }

        &FsItem::Empty => unreachable!(),
    };

    debug!("checking {:?}", full_path);

    match item {
        &FsItem::Dir(_, _) => assert!(chroot.is_dirat(dir_fd, &path)),

        &FsItem::FileLink(_, _, _) |
        &FsItem::DirLink(_, _, _) |
        &FsItem::DeadLink(_, _) => assert!(chroot.is_lnkat(dir_fd, &path)),

        &FsItem::File(_, _) => assert!(chroot.is_regat(dir_fd, &path)),

        &FsItem::Empty => unreachable!(),
    }

    let link = match item {
        &FsItem::FileLink(_, _, _) |
        &FsItem::DirLink(_, _, _) |
        &FsItem::DeadLink(_, _) => Some(
            dir_fd.readlinkat(&path).expect(&format!(
                "failed to read link {:?}",
                full_path
            )),
        ),

        _ => None,
    };

    let target_stat = match item {
        &FsItem::File(_, _) |
        &FsItem::Dir(_, _) => chroot.fstatat(dir_fd, &path),

        &FsItem::FileLink(_, _, _) |
        &FsItem::DirLink(_, _, _) |
        &FsItem::DeadLink(_, _) => {
            chroot.fstatat(dir_fd, link.as_ref().unwrap())
        }

        &FsItem::Empty => unreachable!(),
    };

    let source_stat = chroot.fstatat(dir_fd, &path).expect(&format!(
        "failed to stat component {:?}",
        full_path
    ));

    let full_stat = chroot.fstatat(&root.root_fd, &full_path).expect(&format!(
        "failed to stat path {:?}",
        full_path
    ));

    assert!(::fd::same_file_by_stat(&source_stat, &full_stat));

    let fds = match item {
        &FsItem::Dir(_, _) |
        &FsItem::DirLink(_, _, _) => {
            Some((chroot.openat(dir_fd, &path, 0
                                | libc::O_RDONLY | libc::O_DIRECTORY
                                | libc::O_CLOEXEC)
                  .expect(&format!("failed to open dir component {:?}", full_path)),

                  chroot.openat(dir_fd, &full_path, 0
                                | libc::O_RDONLY | libc::O_DIRECTORY
                                | libc::O_CLOEXEC)
                  .expect(&format!("failed to open dir path {:?}", full_path))))
        }

        &FsItem::DeadLink(_, _) => None,

        &FsItem::File(_, _) |
        &FsItem::FileLink(_, _, _) => {
            Some((chroot.openat(dir_fd, &path, 0
                                | libc::O_RDONLY | libc::O_CLOEXEC)
                  .expect(&format!("failed to open file component {:?}", full_path)),

                  chroot.openat(dir_fd, &full_path, 0
                                | libc::O_RDONLY | libc::O_CLOEXEC)
                  .expect(&format!("failed to open file path {:?}", full_path))))
        }

        &FsItem::Empty => unreachable!(),
    };

    let fd_ref = match fds {
        Some((fd_comp, fd_path)) => {
            assert!(::fd::same_file_by_stat(
                &fd_comp.fstat().unwrap(),
                &fd_path.fstat().unwrap(),
            ));
            Some(fd_comp)
        }

        None => None,
    };

    match item {
        &FsItem::Dir(_, content) => {
            let fd = fd_ref.unwrap();

            for i in content {
                check_fsitem(root, &fd, i);
            }
        }

        &FsItem::File(_, content) |
        &FsItem::FileLink(_, _, content) => {
            let fd = fd_ref.unwrap();
            let mut data = String::new();

            fd.into_file()
                .expect("failed to convert fd")
                .read_to_string(&mut data)
                .expect(&format!("failed to read file {:?}", full_path));

            assert!(target_stat.is_ok());
            assert_eq!(data, content);
        }

        &FsItem::DirLink(_, _, exp) => {
            // assume absolute paths for now
            assert_eq!(exp[0], b'/');
            // the ...dir.join() below does not work with absolute paths
            let exp = OsString::from_vec(exp[1..].to_vec());
            let exp = std::path::Path::new(&exp);
            let target_stat = target_stat.unwrap();

            let fd = chroot.openat(dir_fd, link.as_ref().unwrap(), 0
                                   | libc::O_RDONLY | libc::O_CLOEXEC
                                   | libc::O_DIRECTORY)
                .expect(&format!("failed to open dirlink {:?}", full_path));

            let st_a = fd.fstat().expect(&format!(
                "failed to stat dirlink {:?}",
                full_path
            ));

            let st_b =
                ::fd::Fd::stat(&root.dir.join(exp), false).expect(&format!(
                    "failed to stat reference dir {:?}",
                    exp
                ));

            assert!(::fd::same_file_by_stat(&st_a, &st_b));
            assert!(::fd::same_file_by_stat(&st_a, &target_stat));
        }

        &FsItem::DeadLink(_, _) => {
            //assert!(target_stat.is_err());

            chroot
                .openat(
                    dir_fd,
                    link.as_ref().unwrap(),
                    0 | libc::O_RDONLY | libc::O_CLOEXEC | libc::O_DIRECTORY,
                )
                .expect_err(&format!(
                    "opened dead link {:?} unexpectectly as dir",
                    full_path
                ));

            chroot
                .openat(
                    dir_fd,
                    link.as_ref().unwrap(),
                    0 | libc::O_RDONLY | libc::O_CLOEXEC,
                )
                .expect_err(&format!(
                    "opened dead link {:?} unexpectectly as file",
                    full_path
                ));
        }

        &FsItem::Empty => unreachable!(),
    }
}

#[test]
fn test0() {
    use chroot::Chroot;
    use env_logger;

    let _ = env_logger::init();

    let tmpdir = ::test::create_tmpdir();
    let chroot_path = &tmpdir.path().join("chroot");

    ::test::create_fs(&tmpdir.path(), &TEST_FS_OUTSIDE);
    ::test::create_fs(chroot_path, &TEST_FS_INSIDE);

    let chroot = Chroot::new(chroot_path);
    let root_fd = chroot.root_fd().expect("failed to get chroot fd");

    let root = ChrootedChroot {
        dir: tmpdir.path().to_owned(),
        chroot: chroot,
        root_fd: root_fd,
    };

    check_fsitem(
        &root,
        &root.chroot.root_fd().expect("failed to get chroot fd"),
        &TEST_FS_INSIDE,
    );
}
