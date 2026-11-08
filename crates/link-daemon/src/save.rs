//! Saving a phone's file into the downloads folder: the phone names it, so the name is cleaned, the bytes are staged
//! in a hidden file, and the final name is taken without replacing an existing file.

use std::fs::{self, File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// Bytes of a saved name, extension included.
const MAX_NAME: usize = 120;
const STAGE_PREFIX: &str = ".link-save-";
const STAGE_SUFFIX: &str = ".part";

/// The last path component of `name` without control characters or leading dots or spaces, cut to [`MAX_NAME`]
/// bytes; `photo` when nothing is left.
pub fn safe_name(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or_default();
    let clean: String = base.chars().filter(|c| !c.is_control()).collect();
    let mut clean = clean.trim_matches(|c: char| c == '.' || c.is_whitespace()).to_owned();
    while clean.len() > MAX_NAME {
        clean.pop();
    }
    let clean = clean.trim_end();
    if clean.is_empty() { "photo".to_owned() } else { clean.to_owned() }
}

/// `folder` resolved, when it is an absolute path to an existing directory the daemon's user owns.
pub fn own_folder(folder: &str) -> Result<PathBuf, &'static str> {
    use std::os::unix::fs::MetadataExt as _;
    if !Path::new(folder).is_absolute() {
        return Err("the folder is not an absolute path");
    }
    let resolved = fs::canonicalize(folder).map_err(|_| "the folder does not exist")?;
    let meta = fs::metadata(&resolved).map_err(|_| "the folder does not exist")?;
    if !meta.is_dir() {
        return Err("not a folder");
    }
    if meta.uid() != rustix::process::getuid().as_raw() {
        return Err("the folder belongs to another user");
    }
    Ok(resolved)
}

/// Tags a file saved from a phone with `user.xdg.origin.url` = `link://<device_id>` and `user.andromeda.origin` = the
/// phone's name, for file managers; filesystems without user xattrs keep the file untagged.
pub fn mark_origin(path: &Path, device_id: &str, name: &str) {
    use rustix::fs::{XattrFlags, setxattr};
    let url = format!("link://{device_id}");
    for (key, value) in [("user.xdg.origin.url", url.as_str()), ("user.andromeda.origin", name)] {
        match setxattr(path, key, value.as_bytes(), XattrFlags::empty()) {
            Ok(()) | Err(rustix::io::Errno::NOTSUP) => {}
            Err(error) => log::info!("tagging {} with {key}: {error}", path.display()),
        }
    }
}

/// Deletes the staging files an earlier run left in `dir` when it died.
pub fn sweep(dir: &Path) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    for entry in entries.filter_map(Result::ok) {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with(STAGE_PREFIX) && name.ends_with(STAGE_SUFFIX) {
            drop(fs::remove_file(entry.path()));
        }
    }
}

/// A way to give the staged file its final name without replacing a file: `Err(AlreadyExists)` when the name is taken.
pub type Publish = fn(&Path, &Path) -> io::Result<()>;

fn hard_link(temp: &Path, target: &Path) -> io::Result<()> {
    fs::hard_link(temp, target)
}

/// For filesystems without hard links; the kernel takes the name only if it is free.
fn rename_noreplace(temp: &Path, target: &Path) -> io::Result<()> {
    use rustix::fs::{CWD, RenameFlags, renameat_with};
    renameat_with(CWD, temp, CWD, target, RenameFlags::NOREPLACE).map_err(io::Error::from)
}

/// The last resort, for filesystems with neither: a copy into a file that must not exist yet.
fn copy_new(temp: &Path, target: &Path) -> io::Result<()> {
    let mut out = OpenOptions::new().write(true).create_new(true).open(target)?;
    let copied = io::copy(&mut File::open(temp)?, &mut out).and_then(|_| out.sync_all());
    if copied.is_err() {
        drop(fs::remove_file(target));
    }
    copied
}

pub const PUBLISH: [Publish; 3] = [hard_link, rename_noreplace, copy_new];

/// A file being written inside `dir`; dropping it without [`Staged::publish`] deletes it.
pub struct Staged {
    pub file: File,
    temp: PathBuf,
    dir: PathBuf,
}

impl Staged {
    pub fn create(dir: &Path) -> io::Result<Self> {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        fs::create_dir_all(dir)?;
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let temp = dir.join(format!("{STAGE_PREFIX}{}-{n}{STAGE_SUFFIX}", std::process::id()));
        let file = OpenOptions::new().write(true).create_new(true).open(&temp)?;
        Ok(Self { file, temp, dir: dir.to_owned() })
    }

    /// Gives the file `name`, or `stem (2).ext` and on while that is taken, by the first of [`PUBLISH`] the
    /// filesystem takes; returns the final path.
    pub fn publish(self, name: &str) -> io::Result<PathBuf> {
        self.publish_with(name, &PUBLISH)
    }

    pub fn publish_with(self, name: &str, methods: &[Publish]) -> io::Result<PathBuf> {
        self.file.sync_all()?;
        let (stem, extension) = name.rsplit_once('.').map_or((name, String::new()), |(s, e)| (s, format!(".{e}")));
        let mut last = io::Error::new(io::ErrorKind::Unsupported, "no way to publish");
        for n in 1..10_000 {
            let candidate = if n == 1 { name.to_owned() } else { format!("{stem} ({n}){extension}") };
            let target = self.dir.join(candidate);
            let mut taken = false;
            for method in methods {
                match method(&self.temp, &target) {
                    Ok(()) => return Ok(target),
                    Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                        taken = true;
                        break;
                    }
                    Err(error) => last = error,
                }
            }
            if !taken {
                return Err(last);
            }
        }
        Err(io::Error::new(io::ErrorKind::AlreadyExists, "no free file name"))
    }
}

impl Drop for Staged {
    fn drop(&mut self) {
        drop(fs::remove_file(&self.temp));
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    #[test]
    fn a_hostile_name_becomes_a_plain_file_name() {
        assert_eq!(safe_name("../../.bashrc"), "bashrc");
        assert_eq!(safe_name("..\\..\\x.jpg"), "x.jpg");
        assert_eq!(safe_name(".."), "photo");
        assert_eq!(safe_name(". .bashrc"), "bashrc");
        assert_eq!(safe_name(" . . "), "photo");
        assert_eq!(safe_name("a\nb\u{7}.jpg"), "ab.jpg");
        assert_eq!(safe_name(&"é".repeat(200)).len(), MAX_NAME);
    }

    fn scratch(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!("link-save-{tag}-{}", std::process::id()))
    }

    #[expect(clippy::expect_used, reason = "a failed step fails the test")]
    fn save(dir: &Path, bytes: &[u8], methods: &[Publish]) -> io::Result<PathBuf> {
        let mut staged = Staged::create(dir).expect("stage");
        staged.file.write_all(bytes).expect("write");
        staged.publish_with("a.jpg", methods)
    }

    #[test]
    #[expect(clippy::expect_used, reason = "a failed step fails the test")]
    fn a_saved_file_never_replaces_another() {
        let dir = scratch("keep");
        let (first, second) = (save(&dir, b"one", &PUBLISH), save(&dir, b"two", &PUBLISH));
        let (first, second) = (first.expect("first"), second.expect("second"));
        assert_eq!(fs::read(&first).expect("first"), b"one");
        assert_eq!(second.file_name().and_then(|n| n.to_str()), Some("a (2).jpg"));
        assert_eq!(fs::read(&second).expect("second"), b"two");
        let left = fs::read_dir(&dir).expect("dir").count();
        drop(fs::remove_dir_all(&dir));
        assert_eq!(left, 2, "a staging file was left behind");
    }

    #[test]
    #[expect(clippy::expect_used, reason = "a failed step fails the test")]
    fn a_filesystem_without_links_falls_back_to_a_copy_and_still_never_replaces() {
        fn no_links(_: &Path, _: &Path) -> io::Result<()> {
            Err(io::Error::from(io::ErrorKind::Unsupported))
        }
        let dir = scratch("fallback");
        let methods = [no_links as Publish, no_links, copy_new];
        let first = save(&dir, b"one", &methods).expect("first");
        let second = save(&dir, b"two", &methods).expect("second");
        assert_eq!(fs::read(&first).expect("first"), b"one");
        assert_eq!(fs::read(&second).expect("second"), b"two");
        assert_ne!(first, second);
        assert!(save(&dir, b"three", &[no_links]).is_err(), "a failing method must report, not succeed");
        drop(fs::remove_dir_all(&dir));
    }

    #[test]
    #[expect(clippy::expect_used, reason = "a failed step fails the test")]
    fn only_an_existing_own_directory_is_a_save_folder() {
        let dir = scratch("folder");
        fs::create_dir_all(&dir).expect("dir");
        fs::write(dir.join("file"), b"x").expect("file");
        let path = |name: &str| dir.join(name).to_string_lossy().into_owned();
        let own = own_folder(&dir.to_string_lossy());
        let (file, missing) = (own_folder(&path("file")), own_folder(&path("missing")));
        drop(fs::remove_dir_all(&dir));
        assert_eq!(
            own.ok(),
            fs::canonicalize(std::env::temp_dir()).ok().map(|tmp| tmp.join(dir.file_name().expect("name")))
        );
        assert_eq!(file, Err("not a folder"));
        assert_eq!(missing, Err("the folder does not exist"));
        assert_eq!(own_folder("relative"), Err("the folder is not an absolute path"));
        assert_eq!(own_folder("/"), Err("the folder belongs to another user"));
    }

    #[test]
    #[expect(clippy::expect_used, reason = "a failed step fails the test")]
    fn the_sweep_removes_only_staging_files() {
        let dir = scratch("sweep");
        fs::create_dir_all(&dir).expect("dir");
        for name in [".link-save-1-0.part", "photo.jpg", ".link-save-notes.txt"] {
            fs::write(dir.join(name), b"x").expect("file");
        }
        sweep(&dir);
        let mut left: Vec<_> = fs::read_dir(&dir).expect("dir").filter_map(Result::ok).map(|e| e.file_name()).collect();
        left.sort();
        drop(fs::remove_dir_all(&dir));
        assert_eq!(left, [".link-save-notes.txt", "photo.jpg"]);
    }
}
