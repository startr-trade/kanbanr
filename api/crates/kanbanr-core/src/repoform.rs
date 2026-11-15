//! A working file's repository form: the bytes `git add` would store for it, after git's to-git
//! conversion (`core.autocrlf`, `core.eol`, the `text`, `eol`, `crlf` and `ident` attributes), so a
//! position compares a checkout with line-ending conversion as `git status` and katalogr do
//! (DEF-066, katalogr FEAT-141).
//!
//! The git2 crate has no binding for libgit2's filter API, so the three functions are declared here
//! against the vendored libgit2 git2 already links (1.9.7, `include/git2/filter.h`). This is the
//! only `unsafe` code of the position read; every call is guarded and every allocation released on
//! every path. Driver filters (`filter=<name>`, LFS) are never run: none is registered, so libgit2
//! has nothing to start. `GIT_FILTER_ALLOW_UNSAFE` keeps `core.safecrlf` from failing a read.

use git2::Binding;
use std::borrow::Cow;
use std::ffi::CString;
use std::os::raw::{c_char, c_int};
use std::path::{Path, PathBuf};

#[repr(C)]
struct GitFilterList {
    _p: [u8; 0],
}

/// `GIT_FILTER_TO_ODB`: the working tree to the object store, the direction `git add` filters.
const GIT_FILTER_TO_ODB: c_int = 1;
/// `GIT_FILTER_ALLOW_UNSAFE`: an irreversible CRLF conversion (`core.safecrlf`) is not an error.
const GIT_FILTER_ALLOW_UNSAFE: u32 = 1 << 0;

unsafe extern "C" {
    fn git_filter_list_load(
        out: *mut *mut GitFilterList,
        repo: *mut libgit2_sys::git_repository,
        blob: *mut libgit2_sys::git_blob,
        path: *const c_char,
        mode: c_int,
        flags: u32,
    ) -> c_int;
    fn git_filter_list_apply_to_buffer(
        out: *mut libgit2_sys::git_buf,
        filters: *mut GitFilterList,
        input: *const c_char,
        len: usize,
    ) -> c_int;
    fn git_filter_list_free(filters: *mut GitFilterList);
}

/// Frees a loaded filter list on every path.
struct ListGuard(*mut GitFilterList);

impl Drop for ListGuard {
    fn drop(&mut self) {
        // SAFETY: the pointer is a non-null list git_filter_list_load returned, freed only here.
        unsafe { git_filter_list_free(self.0) }
    }
}

/// Disposes of libgit2's output buffer on every path.
struct BufGuard(libgit2_sys::git_buf);

impl Drop for BufGuard {
    fn drop(&mut self) {
        // SAFETY: the buffer was zeroed before use and only libgit2 filled it; git_buf_dispose
        // accepts a zeroed buffer and frees what libgit2 allocated.
        unsafe { libgit2_sys::git_buf_dispose(&mut self.0) }
    }
}

/// The to-git conversion of one work tree, through vendored libgit2's filter list (DEF-066).
/// The repository is opened on the first conversion only; a root that libgit2 cannot open
/// converts nothing. Not Sync (a git2::Repository is Send, not Sync): one per thread.
pub(crate) struct RepoForm {
    root: PathBuf,
    repo: Option<Option<git2::Repository>>,
    converted: usize,
}

impl RepoForm {
    /// The conversion for the work tree whose top is `root`. Opens nothing yet.
    pub(crate) fn new(root: &Path) -> RepoForm {
        RepoForm {
            root: root.to_path_buf(),
            repo: None,
            converted: 0,
        }
    }

    /// The bytes `git add` would store for the regular file at `rela` (repo-relative,
    /// '/'-separated, UTF-8) whose working content is `bytes`: Borrowed when no filter applies
    /// or the conversion fails, Owned when a filter changed them. Never called for a link.
    /// Counts one conversion per call that reached libgit2.
    pub(crate) fn convert<'a>(&mut self, rela: &str, bytes: &'a [u8]) -> Cow<'a, [u8]> {
        let root = &self.root;
        let Some(repo) = self
            .repo
            .get_or_insert_with(|| git2::Repository::open(root).ok())
        else {
            return Cow::Borrowed(bytes);
        };
        let Ok(path) = CString::new(rela) else {
            return Cow::Borrowed(bytes);
        };
        self.converted += 1;
        let mut list: *mut GitFilterList = std::ptr::null_mut();
        // SAFETY: `repo.raw()` is the pointer of a live Repository that outlives this call; the
        // path is a NUL-terminated CString alive for the call; no blob is passed.
        let rc = unsafe {
            git_filter_list_load(
                &mut list,
                repo.raw(),
                std::ptr::null_mut(),
                path.as_ptr(),
                GIT_FILTER_TO_ODB,
                GIT_FILTER_ALLOW_UNSAFE,
            )
        };
        if rc < 0 || list.is_null() {
            return Cow::Borrowed(bytes);
        }
        let list = ListGuard(list);
        let mut out = BufGuard(libgit2_sys::git_buf {
            ptr: std::ptr::null_mut(),
            reserved: 0,
            size: 0,
        });
        // SAFETY: the list is live (freed by its guard after this); `input` and `len` are one
        // live slice; the output buffer is zeroed and disposed of by its guard.
        let rc = unsafe {
            git_filter_list_apply_to_buffer(
                &mut out.0,
                list.0,
                bytes.as_ptr().cast::<c_char>(),
                bytes.len(),
            )
        };
        if rc < 0 {
            return Cow::Borrowed(bytes);
        }
        let stored: Vec<u8> = if out.0.ptr.is_null() || out.0.size == 0 {
            Vec::new()
        } else {
            // SAFETY: after a successful apply, `ptr[..size]` is libgit2's initialised output,
            // read here before the guard disposes of it.
            unsafe { std::slice::from_raw_parts(out.0.ptr.cast::<u8>(), out.0.size) }.to_vec()
        };
        if stored == bytes {
            Cow::Borrowed(bytes)
        } else {
            Cow::Owned(stored)
        }
    }

    /// How many calls reached libgit2 (for the tests that prove an LF checkout converts nothing).
    pub(crate) fn converted(&self) -> usize {
        self.converted
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coderepo::{git, tests::scratch_repo};

    /// A scratch repository with no line-ending conversion and an empty attributes file pinned in
    /// its own config, so no global setting bends it (libgit2 reads the person's global config).
    fn repo(name: &str) -> PathBuf {
        let dir = scratch_repo(&format!("repoform-{name}"));
        let empty = dir.join(".git/empty-attributes");
        std::fs::write(&empty, "").unwrap();
        git(&dir, &["config", "core.autocrlf", "false"]).unwrap();
        git(
            &dir,
            &["config", "core.attributesFile", empty.to_str().unwrap()],
        )
        .unwrap();
        assert!(
            dir.canonicalize()
                .unwrap()
                .starts_with(std::env::temp_dir().canonicalize().unwrap()),
            "the scratch repository is the test's own"
        );
        dir
    }

    /// R-2: a driver filter, even a required one, is never run: the bytes come back as they are.
    #[test]
    fn driver_filters_are_not_run() {
        let d = repo("driver");
        git(&d, &["config", "filter.up.clean", "tr a-z A-Z"]).unwrap();
        git(&d, &["config", "filter.up.required", "true"]).unwrap();
        std::fs::write(d.join(".gitattributes"), "*.up filter=up\n").unwrap();
        let mut form = RepoForm::new(&d);
        let input = b"abc\n";
        let got = form.convert("a.up", input);
        assert!(matches!(got, Cow::Borrowed(_)), "converted: {got:?}");
        assert_eq!(got.as_ref(), input);
        let _ = std::fs::remove_dir_all(&d);
    }

    /// R-2: with no conversion configured the conversion is the identity, and a root that is not a
    /// repository converts nothing and does not panic.
    #[test]
    fn identity_without_conversion() {
        let d = repo("identity");
        let mut form = RepoForm::new(&d);
        let input = b"a\r\nb\r\n";
        let got = form.convert("a.txt", input);
        assert!(matches!(got, Cow::Borrowed(_)), "converted: {got:?}");
        assert_eq!(got.as_ref(), input);
        let _ = std::fs::remove_dir_all(&d);

        let not_repo = std::env::temp_dir().join(format!(
            "kanbanr-repoform-{}-not-a-repo",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&not_repo);
        std::fs::create_dir_all(&not_repo).unwrap();
        let mut form = RepoForm::new(&not_repo);
        let got = form.convert("a.txt", input);
        assert!(matches!(got, Cow::Borrowed(_)));
        assert_eq!(got.as_ref(), input);
        assert_eq!(form.converted(), 0);
        let _ = std::fs::remove_dir_all(&not_repo);
    }
}
