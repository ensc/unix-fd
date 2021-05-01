use crate::test::FsItem;
use crate::test::FsItem::*;

pub static TEST_FS_OUTSIDE: FsItem =
    Dir(b".", &[
        Dir(b"etc", &[
            File(b"passwd", "outer-etc_passwd"),
            File(b"group", "outer-etc_group"),
        ]),

        Dir(b"tmp", &[
            Dir(b"d0", &[
                Dir(b"d1", &[
                    File(b"f0", "outer-tmp_d0_d1_f0"),
                ]),
            ]),
        ]),

        Dir(b"chroot", &[ Empty ]),
    ]);

pub static TEST_FS_INSIDE: FsItem =
    Dir(b".", &[
        Dir(b"chroot", &[
            File(b"f0", "inner-chroot_f0"),
        ]),

        Dir(b"etc", &[
            File(b"passwd", "inner-etc_passwd"),
            File(b"shadow", "inner-etc_shadow"),
            FileLink(b"lf0", b"../../etc/shadow", "inner-etc_shadow"),
        ]),

        Dir(b"tmp", &[
            Dir(b"d0", &[
                Dir(b"d1", &[
                    File(b"f0", "inner-tmp_d0_d1_f0"),
                    FileLink(b"lf0", b"f0",            "inner-tmp_d0_d1_f0"),
                    FileLink(b"lf1", b"./f0",          "inner-tmp_d0_d1_f0"),
                    FileLink(b"lf2", b"/tmp/d0/d1/f0", "inner-tmp_d0_d1_f0"),

                    DirLink (b"ld0", b"/tmp/ld3/d1/",   b"/chroot/tmp/d0/d1"),
                    FileLink(b"lf3", b"/tmp/ld3/d1/f0", "inner-tmp_d0_d1_f0"),

                    DirLink (b"ld1", b"/tmp/ld5/d1/",   b"/chroot/tmp/d0/d1"),
                    FileLink(b"lf4", b"/tmp/ld5/d1/f0", "inner-tmp_d0_d1_f0"),
                ]),
                Dir(b"d2", &[
                    Empty
                ]),
            ]),
            DirLink(b"ld0", b"d0",         b"/chroot/tmp/d0"),
            DirLink(b"ld1", b"./d0",       b"/chroot/tmp/d0"),
            DirLink(b"ld2", b"/tmp/d0",    b"/chroot/tmp/d0"),
            DirLink(b"ld3", b"../tmp/d0",  b"/chroot/tmp/d0"),
            DirLink(b"ld4", b"../.././../../tmp/d0", b"/chroot/tmp/d0"),
            DirLink(b"ld5", b"../tmp/d0/", b"/chroot/tmp/d0"),

            FileLink(b"lf0", b"/tmp/d0/d1/ld0/..////../ld1/d1/f0", "inner-tmp_d0_d1_f0"),
            FileLink(b"lf1", b"/tmp/d0/d1/ld0/../../ld1/d1/f0",    "inner-tmp_d0_d1_f0"),

            DirLink(b"ld6", b"/",    b"/chroot"),

            File(b"passwd", "inner-tmp_passwd"),

            FileLink(b"lf2", b"passwd",        "inner-tmp_passwd"),
            FileLink(b"lf3", b"/etc/passwd",   "inner-etc_passwd"),
            FileLink(b"lf4", b"/tmp/passwd",   "inner-tmp_passwd"),
            FileLink(b"lf5", b"../etc/passwd", "inner-etc_passwd"),

            FileLink(b"lf6", b"./d0/d1/f0",    "inner-tmp_d0_d1_f0"),

            DeadLink(b"lD0", b"non-existing"),
            DeadLink(b"lD1", b"/etc/group"),
            DeadLink(b"lD2", b"lD2"),
        ]),
        File(b"f0", "inner-f0"),
    ]);
