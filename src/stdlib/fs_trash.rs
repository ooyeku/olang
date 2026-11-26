//! `fs.trash` on Linux: the freedesktop.org Trash specification 1.0, the
//! one GNOME's Files, KDE's Dolphin and `gio trash` read and write, so
//! what olang trashes shows in the desktop's Trash and is restored from
//! there.
//!
//! A trash directory holds `files/` (the items) and `info/` (one
//! `<name>.trashinfo` an item: where it came from and when). The home
//! trash is `$XDG_DATA_HOME/Trash`. An item on another mount goes to
//! that mount's own trash, `$topdir/.Trash/$uid` or `$topdir/.Trash-$uid`,
//! because a move across mounts is a copy.
//!
//! Where the specification leaves a choice this does what GLib's
//! `g_local_file_trash` does: a second `notes.txt` is `notes.2.txt` (the
//! number goes before the first dot), the path is percent-encoded byte by
//! byte with only letters, digits and `-._~/` kept, an item whose mount
//! has no trash that can be used is an error (never a copy into the home
//! trash), and `directorysizes` is not written (the specification makes
//! it optional, and a reader sizes a folder it has no entry for).
//!
//! The work is in [`trash`], which takes its trash directory, user and
//! clock as arguments and is compiled on every Unix so its tests run on a
//! Mac too. Only Linux calls it (`trash_item` in fs.rs).

use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::{self, Write};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

/// What [`trash`] needs from outside the item itself.
pub(crate) struct TrashEnv<'a> {
    /// The home trash directory, `$XDG_DATA_HOME/Trash`.
    pub home_trash: &'a Path,
    /// The user the per-mount trash directories are named for.
    pub uid: u32,
    /// The local time, written as the item's `DeletionDate`.
    pub now: &'a dyn Fn() -> chrono::NaiveDateTime,
    /// The device a path is on, never following a link (`lstat`'s
    /// `st_dev`). A test gives its own to stand for a second mount.
    pub device: &'a dyn Fn(&Path) -> io::Result<u64>,
}

/// `lstat`'s `st_dev`: the device of the path itself, a link's own when
/// it is a link.
pub(crate) fn lstat_device(path: &Path) -> io::Result<u64> {
    fs::symlink_metadata(path).map(|m| m.dev())
}

/// The home trash directory: `$XDG_DATA_HOME/Trash`, and
/// `$HOME/.local/share/Trash` when that variable is unset, empty or not
/// an absolute path (the XDG Base Directory specification ignores a
/// relative one).
pub(crate) fn home_trash_dir(
    xdg_data_home: Option<OsString>,
    home: Option<OsString>,
) -> Result<PathBuf, String> {
    let data = match xdg_data_home {
        Some(d) if Path::new(&d).is_absolute() => PathBuf::from(d),
        _ => match home {
            Some(h) if Path::new(&h).is_absolute() => PathBuf::from(h).join(".local/share"),
            _ => return Err("neither XDG_DATA_HOME nor HOME says where the Trash is".to_string()),
        },
    };
    Ok(data.join("Trash"))
}

/// Move `path` to the trash and answer where it went (its path under
/// `files/`). A link is trashed itself, never what it points to.
pub(crate) fn trash(path: &Path, env: &TrashEnv) -> Result<PathBuf, String> {
    let (parent, name) = locate(path)?;
    let item = parent.join(&name);
    let item_dev = (env.device)(&item).map_err(|e| match e.kind() {
        io::ErrorKind::NotFound => "no such file or folder".to_string(),
        _ => e.to_string(),
    })?;

    // Which trash, and how the item's old place is written in it: the
    // whole path in the home trash, the path from the mount's top
    // directory in a mount's own.
    let (trash_dir, original) = if home_trash_device(env) == Some(item_dev) {
        private_dir_all(env.home_trash)?;
        (env.home_trash.to_path_buf(), item.clone())
    } else {
        let parent_dev = (env.device)(&parent).map_err(|e| e.to_string())?;
        if parent_dev != item_dev {
            return Err("it is where another filesystem is mounted".to_string());
        }
        let top = top_dir(&parent, item_dev, env.device);
        let dir = mount_trash_dir(&top, env.uid)?;
        let relative = item.strip_prefix(&top).unwrap_or(&item).to_path_buf();
        (dir, relative)
    };
    if fs::canonicalize(&trash_dir).is_ok_and(|real| real.starts_with(&item)) {
        return Err("the Trash is inside it".to_string());
    }

    let files_dir = trash_dir.join("files");
    let info_dir = trash_dir.join("info");
    private_dir_all(&files_dir)?;
    private_dir_all(&info_dir)?;

    let info_text = format!(
        "[Trash Info]\nPath={}\nDeletionDate={}\n",
        percent_encode(original.as_os_str()),
        (env.now)().format("%Y-%m-%dT%H:%M:%S")
    );

    // The info file is made first and exclusively, so two programs
    // trashing the same name cannot both take it; the name it gets is
    // the item's name in `files/`.
    for n in 1..=u32::MAX {
        let trashed_name = numbered_name(&name, n);
        let mut info_name = trashed_name.clone();
        info_name.push(".trashinfo");
        let info = info_dir.join(&info_name);
        let mut file = match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&info)
        {
            Ok(f) => f,
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(format!("cannot write '{}': {e}", info.display())),
        };
        // A file left in `files/` with no info file (another program
        // stopped halfway): a rename onto it would replace it, so this
        // name is not free either.
        let dest = files_dir.join(&trashed_name);
        if fs::symlink_metadata(&dest).is_ok() {
            drop(file);
            let _ = fs::remove_file(&info);
            continue;
        }
        if let Err(e) = file.write_all(info_text.as_bytes()) {
            drop(file);
            let _ = fs::remove_file(&info);
            return Err(format!("cannot write '{}': {e}", info.display()));
        }
        drop(file);
        if let Err(e) = fs::rename(&item, &dest) {
            let _ = fs::remove_file(&info);
            return Err(if e.raw_os_error() == Some(libc::EXDEV) {
                format!(
                    "the Trash at '{}' is on another filesystem",
                    trash_dir.display()
                )
            } else {
                e.to_string()
            });
        }
        return Ok(dest);
    }
    Err("the Trash has no name left for it".to_string())
}

/// The item's folder with every link in it resolved, and the item's own
/// name, which is not resolved: a link is trashed as a link.
fn locate(path: &Path) -> Result<(PathBuf, OsString), String> {
    if path.as_os_str().is_empty() {
        return Err("the path is empty".to_string());
    }
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|e| format!("the working directory cannot be read: {e}"))?
            .join(path)
    };
    let missing = |e: io::Error| match e.kind() {
        io::ErrorKind::NotFound => "no such file or folder".to_string(),
        _ => e.to_string(),
    };
    if let (Some(parent), Some(name)) = (absolute.parent(), absolute.file_name()) {
        let parent = fs::canonicalize(parent).map_err(missing)?;
        return Ok((parent, name.to_owned()));
    }
    // A path that ends in `..`, or the root: only the real path has a name.
    let real = fs::canonicalize(&absolute).map_err(missing)?;
    match (real.parent(), real.file_name()) {
        (Some(parent), Some(name)) => Ok((parent.to_path_buf(), name.to_owned())),
        _ => Err("the root folder cannot go to the Trash".to_string()),
    }
}

/// The device the home trash is on: its own when it is there, else that
/// of the nearest folder above it that is (where it will be made). Links
/// are followed here, since the items go where the trash really is.
fn home_trash_device(env: &TrashEnv) -> Option<u64> {
    let mut at = env.home_trash;
    loop {
        if let Ok(real) = fs::canonicalize(at) {
            return (env.device)(&real).ok();
        }
        at = at.parent()?;
    }
}

/// The top directory of the mount `dir` is on: the highest folder above
/// it (or itself) still on `device`. `dir` has no links in it.
pub(crate) fn top_dir(
    dir: &Path,
    device: u64,
    device_of: &dyn Fn(&Path) -> io::Result<u64>,
) -> PathBuf {
    let mut top = dir;
    while let Some(parent) = top.parent() {
        match device_of(parent) {
            Ok(d) if d == device => top = parent,
            _ => break,
        }
    }
    top.to_path_buf()
}

/// A mount's trash for this user: `$topdir/.Trash/$uid` when an
/// administrator made `$topdir/.Trash` (a real folder, not a link, with
/// the sticky bit), else `$topdir/.Trash-$uid`, made when missing.
fn mount_trash_dir(top: &Path, uid: u32) -> Result<PathBuf, String> {
    let shared = top.join(".Trash");
    if let Ok(m) = fs::symlink_metadata(&shared)
        && m.is_dir()
        && m.mode() & 0o1000 != 0
    {
        let mine = shared.join(uid.to_string());
        if own_dir(&mine, uid).is_ok() {
            return Ok(mine);
        }
    }
    let mine = top.join(format!(".Trash-{uid}"));
    match own_dir(&mine, uid) {
        Ok(()) => Ok(mine),
        Err(e) => Err(format!(
            "its filesystem has no Trash and '{}' cannot be used: {e}",
            mine.display()
        )),
    }
}

/// `dir` is a real folder of this user's (not a link), made with mode
/// 0700 when it is missing.
fn own_dir(dir: &Path, uid: u32) -> io::Result<()> {
    if let Err(e) = fs::DirBuilder::new().mode(0o700).create(dir)
        && e.kind() != io::ErrorKind::AlreadyExists
    {
        return Err(e);
    }
    let m = fs::symlink_metadata(dir)?;
    if !m.is_dir() {
        return Err(io::Error::other("it is not a folder"));
    }
    if m.uid() != uid {
        return Err(io::Error::other("it belongs to another user"));
    }
    Ok(())
}

/// Make a folder and those above it that are missing, with mode 0700 (as
/// the XDG Base Directory specification asks for one made on the way to
/// writing a file).
fn private_dir_all(dir: &Path) -> Result<(), String> {
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
        .map_err(|e| format!("cannot make '{}': {e}", dir.display()))
}

/// The name an item gets in the trash on the `n`th try: its own first,
/// then `name.2.ext`, `name.3.ext`, the number before the first dot, or
/// at the end of a name with none. This is GLib's scheme.
fn numbered_name(name: &OsStr, n: u32) -> OsString {
    if n <= 1 {
        return name.to_owned();
    }
    let bytes = name.as_bytes();
    let dot = bytes.iter().position(|&b| b == b'.').unwrap_or(bytes.len());
    let mut out = bytes[..dot].to_vec();
    out.extend_from_slice(format!(".{n}").as_bytes());
    out.extend_from_slice(&bytes[dot..]);
    OsString::from_vec(out)
}

/// A path as the `Path=` line holds it: every byte but the letters, the
/// digits and `-._~` written `%XX`, and each `/` kept.
fn percent_encode(path: &OsStr) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut out = String::new();
    for &b in path.as_bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~' | b'/') {
            out.push(b as char);
        } else {
            out.push('%');
            out.push(HEX[usize::from(b >> 4)] as char);
            out.push(HEX[usize::from(b & 15)] as char);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use tempfile::tempdir;

    const WHEN: &str = "2026-11-26T09:08:07";

    fn fixed_clock() -> chrono::NaiveDateTime {
        chrono::NaiveDateTime::parse_from_str(WHEN, "%Y-%m-%dT%H:%M:%S").unwrap()
    }

    /// A temporary folder with its links resolved (macOS's is under
    /// `/var`, a link to `/private/var`), so a test can say the path it
    /// expects in `Path=`.
    fn scratch() -> (tempfile::TempDir, PathBuf) {
        let dir = tempdir().unwrap();
        let real = fs::canonicalize(dir.path()).unwrap();
        (dir, real)
    }

    fn uid_of(path: &Path) -> u32 {
        fs::metadata(path).unwrap().uid()
    }

    /// Trash into `root/Trash` as the home trash, every path on one device.
    fn trash_in(root: &Path, path: &Path) -> Result<PathBuf, String> {
        let home_trash = root.join("Trash");
        trash(
            path,
            &TrashEnv {
                home_trash: &home_trash,
                uid: uid_of(root),
                now: &fixed_clock,
                device: &lstat_device,
            },
        )
    }

    fn info_of(root: &Path, name: &str) -> String {
        fs::read_to_string(root.join("Trash/info").join(format!("{name}.trashinfo"))).unwrap()
    }

    fn expected_info(path: &str) -> String {
        format!("[Trash Info]\nPath={path}\nDeletionDate={WHEN}\n")
    }

    fn names_in(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn test_trash_a_file() {
        let (_keep, root) = scratch();
        let file = root.join("notes.txt");
        fs::write(&file, "kept").unwrap();

        let to = trash_in(&root, &file).unwrap();

        assert_eq!(to, root.join("Trash/files/notes.txt"));
        assert!(!file.exists());
        assert_eq!(fs::read_to_string(&to).unwrap(), "kept");
        assert_eq!(
            info_of(&root, "notes.txt"),
            expected_info(&format!("{}/notes.txt", root.display()))
        );
        // The trash and its two folders are the user's alone.
        for dir in ["Trash", "Trash/files", "Trash/info"] {
            let mode = fs::metadata(root.join(dir)).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o700, "{dir}");
        }
    }

    #[test]
    fn test_trash_a_folder_with_contents() {
        let (_keep, root) = scratch();
        let folder = root.join("project");
        fs::create_dir_all(folder.join("src/deep")).unwrap();
        fs::write(folder.join("readme.md"), "top").unwrap();
        fs::write(folder.join("src/deep/main.ol"), "inside").unwrap();

        let to = trash_in(&root, &folder).unwrap();

        assert_eq!(to, root.join("Trash/files/project"));
        assert!(!folder.exists());
        assert_eq!(fs::read_to_string(to.join("readme.md")).unwrap(), "top");
        assert_eq!(
            fs::read_to_string(to.join("src/deep/main.ol")).unwrap(),
            "inside"
        );
        assert_eq!(
            info_of(&root, "project"),
            expected_info(&format!("{}/project", root.display()))
        );
    }

    #[test]
    fn test_trash_a_name_already_in_the_trash() {
        let (_keep, root) = scratch();
        let file = root.join("report.tar.gz");
        let folder = root.join("build");
        let mut went = Vec::new();
        for round in ["first", "second", "third"] {
            fs::write(&file, round).unwrap();
            went.push(trash_in(&root, &file).unwrap());
            fs::create_dir(&folder).unwrap();
            fs::write(folder.join("out"), round).unwrap();
            went.push(trash_in(&root, &folder).unwrap());
        }

        let files = root.join("Trash/files");
        assert_eq!(
            went,
            vec![
                files.join("report.tar.gz"),
                files.join("build"),
                files.join("report.2.tar.gz"),
                files.join("build.2"),
                files.join("report.3.tar.gz"),
                files.join("build.3"),
            ]
        );
        // Each kept what it had, and each info file names the one
        // original path.
        assert_eq!(fs::read_to_string(&went[0]).unwrap(), "first");
        assert_eq!(fs::read_to_string(&went[2]).unwrap(), "second");
        assert_eq!(fs::read_to_string(&went[4]).unwrap(), "third");
        assert_eq!(fs::read_to_string(went[5].join("out")).unwrap(), "third");
        let original = expected_info(&format!("{}/report.tar.gz", root.display()));
        for name in ["report.tar.gz", "report.2.tar.gz", "report.3.tar.gz"] {
            assert_eq!(info_of(&root, name), original, "{name}");
        }
        assert_eq!(
            names_in(&root.join("Trash/info")),
            vec![
                "build.2.trashinfo",
                "build.3.trashinfo",
                "build.trashinfo",
                "report.2.tar.gz.trashinfo",
                "report.3.tar.gz.trashinfo",
                "report.tar.gz.trashinfo",
            ]
        );
    }

    #[test]
    fn test_numbered_names_are_glibs() {
        let name = |s: &str, n| numbered_name(OsStr::new(s), n).into_string().unwrap();
        assert_eq!(name("notes.txt", 1), "notes.txt");
        assert_eq!(name("notes.txt", 2), "notes.2.txt");
        assert_eq!(name("a.tar.gz", 12), "a.12.tar.gz");
        assert_eq!(name("Makefile", 2), "Makefile.2");
        assert_eq!(name(".profile", 3), ".3.profile");
    }

    #[test]
    fn test_trash_names_that_need_encoding() {
        let (_keep, root) = scratch();
        let folder = root.join("my files");
        fs::create_dir(&folder).unwrap();
        let cases = [
            ("two words.txt", "two%20words.txt"),
            ("100% done.txt", "100%25%20done.txt"),
            (
                "naïve été 日本.txt",
                "na%C3%AFve%20%C3%A9t%C3%A9%20%E6%97%A5%E6%9C%AC.txt",
            ),
            ("line\nbreak.txt", "line%0Abreak.txt"),
            ("a+b=c&d;e#f?.txt", "a%2Bb%3Dc%26d%3Be%23f%3F.txt"),
            ("kept-._~.txt", "kept-._~.txt"),
        ];
        for (name, encoded) in cases {
            let file = folder.join(name);
            fs::write(&file, name).unwrap();

            let to = trash_in(&root, &file).unwrap();

            assert_eq!(to, root.join("Trash/files").join(name));
            assert_eq!(fs::read_to_string(&to).unwrap(), name);
            let info = info_of(&root, name);
            // The slashes stay, the folder's space is encoded, and the
            // file is three lines whatever the name holds.
            assert_eq!(
                info,
                expected_info(&format!("{}/my%20files/{encoded}", root.display())),
                "{name:?}"
            );
            assert_eq!(info.lines().count(), 3, "{name:?}");
        }
    }

    #[test]
    fn test_percent_encode_takes_bytes_that_are_not_text() {
        let raw = OsStr::from_bytes(b"/tmp/caf\xe9 \xff");
        assert_eq!(percent_encode(raw), "/tmp/caf%E9%20%FF");
    }

    #[test]
    fn test_trash_a_symbolic_link_not_its_target() {
        let (_keep, root) = scratch();
        let target = root.join("target.txt");
        fs::write(&target, "stays").unwrap();
        let link = root.join("link.txt");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        let target_dir = root.join("real");
        fs::create_dir(&target_dir).unwrap();
        fs::write(target_dir.join("inside"), "stays too").unwrap();
        let dir_link = root.join("shortcut");
        std::os::unix::fs::symlink(&target_dir, &dir_link).unwrap();
        let dangling = root.join("dangling");
        std::os::unix::fs::symlink(root.join("nowhere"), &dangling).unwrap();

        for (link, name) in [
            (&link, "link.txt"),
            (&dir_link, "shortcut"),
            (&dangling, "dangling"),
        ] {
            let to = trash_in(&root, link).unwrap();
            assert_eq!(to, root.join("Trash/files").join(name));
            assert!(fs::symlink_metadata(&to).unwrap().file_type().is_symlink());
            assert!(fs::symlink_metadata(link).is_err(), "{name} is gone");
            assert_eq!(
                info_of(&root, name),
                expected_info(&format!("{}/{name}", root.display()))
            );
        }
        assert_eq!(fs::read_to_string(&target).unwrap(), "stays");
        assert_eq!(
            fs::read_to_string(target_dir.join("inside")).unwrap(),
            "stays too"
        );
    }

    #[test]
    fn test_trash_through_a_linked_folder_records_the_real_path() {
        let (_keep, root) = scratch();
        fs::create_dir(root.join("real")).unwrap();
        fs::write(root.join("real/a.txt"), "a").unwrap();
        std::os::unix::fs::symlink(root.join("real"), root.join("via")).unwrap();

        let to = trash_in(&root, &root.join("via/a.txt")).unwrap();

        assert_eq!(to, root.join("Trash/files/a.txt"));
        assert!(!root.join("real/a.txt").exists());
        assert_eq!(
            info_of(&root, "a.txt"),
            expected_info(&format!("{}/real/a.txt", root.display()))
        );
    }

    #[test]
    fn test_trash_a_missing_path() {
        let (_keep, root) = scratch();

        let e = trash_in(&root, &root.join("never-was.txt")).unwrap_err();
        assert_eq!(e, "no such file or folder");
        let e = trash_in(&root, &root.join("no-folder/never-was.txt")).unwrap_err();
        assert_eq!(e, "no such file or folder");
        let e = trash_in(&root, Path::new("")).unwrap_err();
        assert_eq!(e, "the path is empty");

        // Nothing was reserved for it.
        let info = root.join("Trash/info");
        assert!(!info.exists() || names_in(&info).is_empty());
    }

    #[test]
    fn test_trash_skips_an_info_file_left_without_its_file() {
        let (_keep, root) = scratch();
        let info = root.join("Trash/info");
        fs::create_dir_all(&info).unwrap();
        fs::create_dir_all(root.join("Trash/files")).unwrap();
        let stale =
            "[Trash Info]\nPath=/somewhere/else/data.csv\nDeletionDate=2020-01-01T00:00:00\n";
        fs::write(info.join("data.csv.trashinfo"), stale).unwrap();
        let file = root.join("data.csv");
        fs::write(&file, "1,2").unwrap();

        let to = trash_in(&root, &file).unwrap();

        assert_eq!(to, root.join("Trash/files/data.2.csv"));
        assert_eq!(
            info_of(&root, "data.csv"),
            stale,
            "the other's is untouched"
        );
        assert_eq!(
            info_of(&root, "data.2.csv"),
            expected_info(&format!("{}/data.csv", root.display()))
        );
    }

    #[test]
    fn test_trash_skips_a_file_left_without_its_info_file() {
        let (_keep, root) = scratch();
        let files = root.join("Trash/files");
        fs::create_dir_all(&files).unwrap();
        fs::write(files.join("data.csv"), "someone else's").unwrap();
        let file = root.join("data.csv");
        fs::write(&file, "1,2").unwrap();

        let to = trash_in(&root, &file).unwrap();

        assert_eq!(to, files.join("data.2.csv"));
        assert_eq!(
            fs::read_to_string(files.join("data.csv")).unwrap(),
            "someone else's"
        );
        assert_eq!(
            names_in(&root.join("Trash/info")),
            vec!["data.2.csv.trashinfo"]
        );
    }

    #[test]
    fn test_a_failed_move_leaves_no_info_file() {
        let (_keep, root) = scratch();
        // A folder that holds the trash cannot be moved into it.
        let e = trash_in(&root, &root).unwrap_err();
        assert_eq!(e, "the Trash is inside it");
        assert!(root.exists());

        // The move itself refused: the item's folder cannot be written.
        let locked = root.join("locked");
        fs::create_dir(&locked).unwrap();
        fs::write(locked.join("stuck.txt"), "x").unwrap();
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o500)).unwrap();
        let result = trash_in(&root, &locked.join("stuck.txt"));
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o700)).unwrap();
        // Root may move it all the same (a container's tests run as root).
        if uid_of(&root) != 0 {
            assert!(result.is_err(), "{result:?}");
            assert!(locked.join("stuck.txt").exists());
            assert!(names_in(&root.join("Trash/info")).is_empty());
            assert!(names_in(&root.join("Trash/files")).is_empty());
        }
    }

    #[test]
    fn test_trash_a_relative_path() {
        let (_keep, root) = scratch();
        // The working directory belongs to the whole test process, so
        // the path is made relative to it, not the directory changed.
        let cwd = fs::canonicalize(std::env::current_dir().unwrap()).unwrap();
        let mut up = PathBuf::new();
        for _ in cwd.components().skip(1) {
            up.push("..");
        }
        let file = root.join("rel.txt");
        fs::write(&file, "r").unwrap();
        let relative = up.join(file.strip_prefix("/").unwrap());
        assert!(relative.is_relative());

        let to = trash_in(&root, &relative).unwrap();

        assert_eq!(to, root.join("Trash/files/rel.txt"));
        assert_eq!(
            info_of(&root, "rel.txt"),
            expected_info(&format!("{}/rel.txt", root.display()))
        );
    }

    // ---- another mount, stood for by a device function ----

    /// Everything under `volume` is on device 2, the rest on device 1.
    fn two_devices(volume: &Path) -> impl Fn(&Path) -> io::Result<u64> + '_ {
        move |p: &Path| {
            fs::symlink_metadata(p)?;
            Ok(if p.starts_with(volume) { 2 } else { 1 })
        }
    }

    #[test]
    fn test_top_dir_walks_up_to_the_mount() {
        let device = |p: &Path| -> io::Result<u64> {
            Ok(if p.starts_with("/mnt/usb") {
                7
            } else if p.starts_with("/mnt") {
                3
            } else {
                1
            })
        };
        let top = |dir: &str, dev| top_dir(Path::new(dir), dev, &device);
        assert_eq!(top("/mnt/usb/photos/2026", 7), Path::new("/mnt/usb"));
        assert_eq!(top("/mnt/usb", 7), Path::new("/mnt/usb"));
        assert_eq!(top("/mnt/other", 3), Path::new("/mnt"));
        assert_eq!(top("/home/me/docs", 1), Path::new("/"));
        assert_eq!(top("/", 1), Path::new("/"));
        // A folder that cannot be read ends the walk below it.
        let refuses = |p: &Path| -> io::Result<u64> {
            if p == Path::new("/a") {
                Err(io::Error::other("no"))
            } else {
                Ok(1)
            }
        };
        assert_eq!(top_dir(Path::new("/a/b/c"), 1, &refuses), Path::new("/a/b"));
    }

    #[test]
    fn test_trash_on_another_mount_uses_its_own_trash() {
        let (_keep, root) = scratch();
        let volume = root.join("volume");
        fs::create_dir_all(volume.join("photos/2026")).unwrap();
        let file = volume.join("photos/2026/a b.jpg");
        fs::write(&file, "jpeg").unwrap();
        let uid = uid_of(&root);
        let device = two_devices(&volume);
        let home_trash = root.join("home/Trash");
        let env = TrashEnv {
            home_trash: &home_trash,
            uid,
            now: &fixed_clock,
            device: &device,
        };

        let to = trash(&file, &env).unwrap();

        // `.Trash-$uid` at the mount's top, made 0700, and the path
        // written from that top, not from the root.
        let mount_trash = volume.join(format!(".Trash-{uid}"));
        assert_eq!(to, mount_trash.join("files/a b.jpg"));
        assert_eq!(fs::read_to_string(&to).unwrap(), "jpeg");
        assert!(!file.exists());
        assert_eq!(
            fs::read_to_string(mount_trash.join("info/a b.jpg.trashinfo")).unwrap(),
            expected_info("photos/2026/a%20b.jpg")
        );
        let mode = fs::metadata(&mount_trash).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o700);
        // The home trash was not made for an item that did not go there.
        assert!(!home_trash.exists());

        // A second item of that name on the mount gets the next name there.
        fs::write(&file, "again").unwrap();
        let to = trash(&file, &env).unwrap();
        assert_eq!(to, mount_trash.join("files/a b.2.jpg"));
    }

    #[test]
    fn test_trash_on_another_mount_prefers_the_administrators_trash() {
        let (_keep, root) = scratch();
        let volume = root.join("volume");
        fs::create_dir_all(volume.join(".Trash")).unwrap();
        fs::set_permissions(volume.join(".Trash"), fs::Permissions::from_mode(0o1777)).unwrap();
        let file = volume.join("doc.txt");
        fs::write(&file, "d").unwrap();
        let uid = uid_of(&root);
        let device = two_devices(&volume);
        let home_trash = root.join("home/Trash");
        let env = TrashEnv {
            home_trash: &home_trash,
            uid,
            now: &fixed_clock,
            device: &device,
        };

        let to = trash(&file, &env).unwrap();

        let mine = volume.join(".Trash").join(uid.to_string());
        assert_eq!(to, mine.join("files/doc.txt"));
        assert_eq!(
            fs::read_to_string(mine.join("info/doc.txt.trashinfo")).unwrap(),
            expected_info("doc.txt")
        );
        assert!(!volume.join(format!(".Trash-{uid}")).exists());
    }

    #[test]
    fn test_trash_on_another_mount_distrusts_a_bad_shared_trash() {
        let uid_dir = |volume: &Path, uid: u32| volume.join(format!(".Trash-{uid}"));

        // No sticky bit: anyone could swap another user's folder in it.
        let (_keep, root) = scratch();
        let volume = root.join("volume");
        fs::create_dir_all(volume.join(".Trash")).unwrap();
        fs::set_permissions(volume.join(".Trash"), fs::Permissions::from_mode(0o777)).unwrap();
        fs::write(volume.join("a.txt"), "a").unwrap();
        let uid = uid_of(&root);
        let device = two_devices(&volume);
        let home_trash = root.join("home/Trash");
        let env = TrashEnv {
            home_trash: &home_trash,
            uid,
            now: &fixed_clock,
            device: &device,
        };
        let to = trash(&volume.join("a.txt"), &env).unwrap();
        assert_eq!(to, uid_dir(&volume, uid).join("files/a.txt"));
        assert!(names_in(&volume.join(".Trash")).is_empty());

        // A link named `.Trash`, sticky folder behind it or not.
        let (_keep, root) = scratch();
        let volume = root.join("volume");
        fs::create_dir_all(volume.join("elsewhere")).unwrap();
        fs::set_permissions(volume.join("elsewhere"), fs::Permissions::from_mode(0o1777)).unwrap();
        std::os::unix::fs::symlink(volume.join("elsewhere"), volume.join(".Trash")).unwrap();
        fs::write(volume.join("b.txt"), "b").unwrap();
        let device = two_devices(&volume);
        let home_trash = root.join("home/Trash");
        let env = TrashEnv {
            home_trash: &home_trash,
            uid,
            now: &fixed_clock,
            device: &device,
        };
        let to = trash(&volume.join("b.txt"), &env).unwrap();
        assert_eq!(to, uid_dir(&volume, uid).join("files/b.txt"));
        assert!(names_in(&volume.join("elsewhere")).is_empty());
    }

    #[test]
    fn test_trash_on_another_mount_with_no_usable_trash_is_an_error() {
        let (_keep, root) = scratch();
        let volume = root.join("volume");
        fs::create_dir_all(&volume).unwrap();
        let file = volume.join("keep.txt");
        fs::write(&file, "k").unwrap();
        let uid = uid_of(&root);
        // `.Trash-$uid` is taken by something that is not a folder.
        let taken = volume.join(format!(".Trash-{uid}"));
        fs::write(&taken, "not a folder").unwrap();
        let device = two_devices(&volume);
        let home_trash = root.join("home/Trash");
        let env = TrashEnv {
            home_trash: &home_trash,
            uid,
            now: &fixed_clock,
            device: &device,
        };

        let e = trash(&file, &env).unwrap_err();

        assert_eq!(
            e,
            format!(
                "its filesystem has no Trash and '{}' cannot be used: it is not a folder",
                taken.display()
            )
        );
        // Not copied to the home trash, and not removed.
        assert_eq!(fs::read_to_string(&file).unwrap(), "k");
        assert!(!home_trash.exists());

        // A mount's own top folder is not something to trash.
        let e = trash(&volume, &env).unwrap_err();
        assert_eq!(e, "it is where another filesystem is mounted");
    }

    #[test]
    fn test_home_trash_dir_reads_the_environment() {
        let os = |s: &str| Some(OsString::from(s));
        assert_eq!(
            home_trash_dir(os("/data/xdg"), os("/home/me")).unwrap(),
            Path::new("/data/xdg/Trash")
        );
        assert_eq!(
            home_trash_dir(None, os("/home/me")).unwrap(),
            Path::new("/home/me/.local/share/Trash")
        );
        // Empty or relative is unset.
        for unset in ["", "relative/data"] {
            assert_eq!(
                home_trash_dir(os(unset), os("/home/me")).unwrap(),
                Path::new("/home/me/.local/share/Trash")
            );
        }
        assert!(home_trash_dir(None, None).is_err());
    }
}
