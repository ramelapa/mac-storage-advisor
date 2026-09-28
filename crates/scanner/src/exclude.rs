//! Exclusion rules used by the scanner.
//!
//! # Rules
//!
//! Patterns are classified after trimming whitespace and a single trailing `/`:
//!
//! 1. **Glob** — the pattern contains `*` or `?`.
//!    - No `/`: the pattern is matched against the entry's own file name.
//!      `*` matches any characters inside that one name, `?` matches one
//!      character. `**` inside a single name is just two `*` wildcards.
//!    - With `/`: the pattern is split on `/`. A segment that is exactly `**`
//!      matches any sequence of components (including empty). Other segments
//!      use `*` and `?` inside one component.
//!      An absolute pattern (leading `/`) must match the whole path from the
//!      root. A relative pattern matches any suffix of the path's components.
//! 2. **Path prefix** — the pattern contains `/` and no glob characters.
//!    Relative patterns are joined to the scan root. The path is normalized
//!    lexically (`.` and `..` removed) and is **not** canonicalized, so
//!    symlinks are not resolved. The exclusion matches that path and every
//!    descendant. Comparison is component-wise, so `/usr` does not match
//!    `/usr2`.
//! 3. **Name** — no `/` and no glob characters. Matches an entry whose own
//!    file name equals the string. A matching directory is not descended into.
//!    This does not match every ancestor: excluding `bin` skips a directory
//!    named `bin`, not `/usr/bin` when the entry's name is something else,
//!    and not a project file that merely lives under a parent named `bin`
//!    unless that parent itself was skipped.
//!
//! The scan root is never excluded by `filter` depth 0. An exclusion that
//! normalizes to the scan root is a fatal error.
//!
//! # Protected macOS prefixes
//!
//! `/System`, `/private`, `/bin`, `/sbin`, `/usr`, and `/Library` are prefix
//! exclusions **only when they are proper descendants of the scan root**
//! (scanning `/` skips them; scanning `~/Downloads` does not). If the scan
//! root *is* exactly one of those paths, the scan is refused unless
//! `allow_protected_roots` is set. An explicit root that merely lives under a
//! protected prefix (for example `/usr/local/myproject`) is scanned.
//!
//! Matching is case-sensitive. There is no character-class syntax.

use std::path::{Component, Path, PathBuf};

use mac_storage_common::ScanTarget;

use crate::ScanFatal;

/// Fixed macOS prefixes. Not a complete privacy policy; see `docs/macos-filesystem.md`.
pub const PROTECTED_MACOS_PREFIXES: &[&str] =
    &["/System", "/private", "/bin", "/sbin", "/usr", "/Library"];

const MAX_PATH_COMPONENTS: usize = 512;
const MAX_GLOB_SEGMENTS: usize = 64;

#[derive(Debug, Clone)]
enum Compiled {
    Name(String),
    Prefix(PathBuf),
    Glob(String),
}

#[derive(Debug, Clone)]
pub struct ExclusionSet {
    items: Vec<Compiled>,
}

impl ExclusionSet {
    pub fn compile(root: &Path, target: &ScanTarget) -> Result<Self, ScanFatal> {
        let mut items = Vec::new();
        for raw in &target.exclusions {
            if let Some(compiled) = compile_one(root, raw, target.redact_paths)? {
                items.push(compiled);
            }
        }
        for prefix in protected_prefixes_under(root) {
            items.push(Compiled::Prefix(prefix));
        }
        Ok(Self { items })
    }

    pub fn matches(&self, path: &Path) -> bool {
        let normalized = normalize_lexical(path);
        self.items.iter().any(|item| match item {
            Compiled::Name(name) => normalized
                .file_name()
                .is_some_and(|file_name| file_name == std::ffi::OsStr::new(name)),
            Compiled::Prefix(prefix) => normalized.starts_with(prefix),
            Compiled::Glob(pattern) => glob_matches(pattern, &normalized),
        })
    }

    /// Patterns that should be stored with the scan: user strings plus applied protected prefixes.
    pub fn persisted_patterns(root: &Path, user_patterns: &[String]) -> Vec<String> {
        let mut out = user_patterns.to_vec();
        for prefix in protected_prefixes_under(root) {
            out.push(prefix.to_string_lossy().into_owned());
        }
        out
    }
}

pub fn is_exact_protected_root(path: &Path) -> bool {
    let path = normalize_lexical(path);
    PROTECTED_MACOS_PREFIXES
        .iter()
        .any(|prefix| path == Path::new(prefix))
}

/// True when `path` is a protected macOS prefix or a descendant of one.
pub fn is_protected_path(path: &Path) -> bool {
    let path = normalize_lexical(path);
    PROTECTED_MACOS_PREFIXES.iter().any(|prefix| {
        let prefix = Path::new(prefix);
        path == prefix || path.starts_with(prefix)
    })
}

pub fn protected_prefixes_under(root: &Path) -> Vec<PathBuf> {
    let root = normalize_lexical(root);
    PROTECTED_MACOS_PREFIXES
        .iter()
        .filter_map(|prefix| {
            let prefix = PathBuf::from(prefix);
            if prefix.starts_with(&root) && prefix != root {
                Some(prefix)
            } else {
                None
            }
        })
        .collect()
}

pub fn normalize_lexical(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    if out.as_os_str().is_empty() {
        PathBuf::from(".")
    } else {
        out
    }
}

fn compile_one(root: &Path, raw: &str, redact: bool) -> Result<Option<Compiled>, ScanFatal> {
    let trimmed = raw.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return Ok(None);
    }
    if trimmed == "." || trimmed == ".." {
        return Err(ScanFatal::InvalidExclusion {
            pattern: raw.to_owned(),
            reason: "`.` and `..` are not valid exclusions".to_owned(),
        });
    }

    let is_glob = trimmed.contains('*') || trimmed.contains('?');
    if is_glob {
        return Ok(Some(Compiled::Glob(trimmed.to_owned())));
    }

    if trimmed.contains('/') {
        let pattern_path = Path::new(trimmed);
        let joined = if pattern_path.is_absolute() {
            pattern_path.to_path_buf()
        } else {
            root.join(pattern_path)
        };
        let norm = normalize_lexical(&joined);
        if norm == root {
            return Err(ScanFatal::ExclusionMatchesRoot {
                pattern: raw.to_owned(),
            });
        }
        if !norm.starts_with(root) && !root.starts_with(&norm) {
            if redact {
                tracing::warn!("exclusion is outside the scan root and will not match entries");
            } else {
                tracing::warn!(
                    exclusion = %norm.display(),
                    "exclusion is outside the scan root and will not match entries"
                );
            }
        }
        return Ok(Some(Compiled::Prefix(norm)));
    }

    Ok(Some(Compiled::Name(trimmed.to_owned())))
}

fn glob_matches(pattern: &str, path: &Path) -> bool {
    let is_abs = pattern.starts_with('/');
    let pat_segs: Vec<&str> = pattern
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect();
    if pat_segs.len() > MAX_GLOB_SEGMENTS {
        return false;
    }
    if !pattern.contains('/') {
        let Some(name) = path.file_name() else {
            return false;
        };
        let name = name.to_string_lossy();
        return match_segment(pattern, &name);
    }

    let text_owned: Vec<String> = path
        .components()
        .filter_map(|component| match component {
            Component::Normal(segment) => Some(segment.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect();
    if text_owned.len() > MAX_PATH_COMPONENTS {
        return false;
    }
    let text: Vec<&str> = text_owned.iter().map(String::as_str).collect();
    if is_abs {
        match_components(&pat_segs, &text)
    } else {
        (0..=text.len()).any(|start| match_components(&pat_segs, &text[start..]))
    }
}

fn match_components(pat: &[&str], text: &[&str]) -> bool {
    fn rec(
        pi: usize,
        ti: usize,
        pat: &[&str],
        text: &[&str],
        memo: &mut [Vec<Option<bool>>],
    ) -> bool {
        if let Some(cached) = memo[pi][ti] {
            return cached;
        }
        let matched = if pi == pat.len() {
            ti == text.len()
        } else if pat[pi] == "**" {
            let zero = rec(pi + 1, ti, pat, text, memo);
            zero || (ti < text.len() && rec(pi, ti + 1, pat, text, memo))
        } else if ti == text.len() {
            false
        } else {
            match_segment(pat[pi], text[ti]) && rec(pi + 1, ti + 1, pat, text, memo)
        };
        memo[pi][ti] = Some(matched);
        matched
    }

    let mut memo = vec![vec![None; text.len() + 1]; pat.len() + 1];
    rec(0, 0, pat, text, &mut memo)
}

fn match_segment(pat: &str, text: &str) -> bool {
    let pattern = pat.as_bytes();
    let text = text.as_bytes();
    let n = pattern.len();
    let m = text.len();
    if n > 1024 || m > 4096 {
        return false;
    }
    let mut prev = vec![false; m + 1];
    let mut curr = vec![false; m + 1];
    prev[0] = true;
    for i in 1..=n {
        curr[0] = pattern[i - 1] == b'*' && prev[0];
        for j in 1..=m {
            curr[j] = match pattern[i - 1] {
                b'*' => prev[j] || curr[j - 1],
                b'?' => prev[j - 1],
                byte => prev[j - 1] && byte == text[j - 1],
            };
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[m]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(root: &str, patterns: &[&str]) -> ExclusionSet {
        let target = ScanTarget {
            root: PathBuf::from(root),
            exclusions: patterns.iter().map(|p| (*p).to_owned()).collect(),
            min_logical_size: 0,
            threads: 1,
            allow_protected_roots: false,
            redact_paths: false,
        };
        ExclusionSet::compile(Path::new(root), &target).unwrap()
    }

    #[test]
    fn name_exclusion_matches_the_entry_name_only() {
        let rules = set("/proj", &["bin"]);
        assert!(rules.matches(Path::new("/proj/bin")));
        assert!(rules.matches(Path::new("/proj/src/bin")));
        assert!(!rules.matches(Path::new("/proj/src/main.rs")));
        assert!(!rules.matches(Path::new("/proj/src/binary")));
    }

    #[test]
    fn prefix_is_component_wise() {
        let rules = set("/tmp/fixture", &["/usr"]);
        assert!(rules.matches(Path::new("/usr")));
        assert!(rules.matches(Path::new("/usr/bin/ls")));
        assert!(!rules.matches(Path::new("/usr2")));
        assert!(!rules.matches(Path::new("/usr2/bin")));
        assert!(!rules.matches(Path::new("/tmp/fixture/bin")));
    }

    #[test]
    fn relative_prefix_is_joined_to_the_root() {
        let rules = set("/tmp/fixture", &["sub/skip"]);
        assert!(rules.matches(Path::new("/tmp/fixture/sub/skip")));
        assert!(rules.matches(Path::new("/tmp/fixture/sub/skip/a.txt")));
        assert!(!rules.matches(Path::new("/tmp/fixture/sub/skip2")));
        assert!(!rules.matches(Path::new("/tmp/fixture/sub/keep.txt")));
    }

    #[test]
    fn exclusion_equal_to_the_root_is_fatal() {
        let target = ScanTarget {
            root: PathBuf::from("/tmp/fixture"),
            exclusions: vec!["/tmp/fixture".into()],
            min_logical_size: 0,
            threads: 1,
            allow_protected_roots: false,
            redact_paths: false,
        };
        let err = ExclusionSet::compile(Path::new("/tmp/fixture"), &target).unwrap_err();
        assert!(matches!(err, ScanFatal::ExclusionMatchesRoot { .. }));
    }

    #[test]
    fn glob_file_name_and_path_patterns() {
        let names = set("/tmp/fixture", &["*.dmg"]);
        assert!(names.matches(Path::new("/tmp/fixture/photo.dmg")));
        assert!(names.matches(Path::new("/Users/me/Downloads/sub/a.dmg")));
        assert!(!names.matches(Path::new("/tmp/fixture/photo.dmg.bak")));

        let paths = set("/tmp/fixture", &["Downloads/*.dmg", "/tmp/x/*.txt"]);
        assert!(paths.matches(Path::new("/Users/me/Downloads/a.dmg")));
        assert!(!paths.matches(Path::new("/Users/me/Downloads/sub/a.dmg")));
        assert!(paths.matches(Path::new("/tmp/x/a.txt")));
        assert!(!paths.matches(Path::new("/tmp/x/sub/a.txt")));
        assert!(!paths.matches(Path::new("/tmp/fixture/photo.dmg")));
    }

    #[test]
    fn double_star_crosses_directories() {
        let rules = set("/tmp", &["/tmp/**/*.txt", "**/*.log"]);
        assert!(rules.matches(Path::new("/tmp/a/b/c.txt")));
        assert!(!rules.matches(Path::new("/tmp/a/b/c.md")));
        assert!(rules.matches(Path::new("/var/log/app.log")));
        assert!(rules.matches(Path::new("/app.log")));
    }

    #[test]
    fn question_mark_matches_one_character() {
        let rules = set("/tmp", &["foo?"]);
        assert!(rules.matches(Path::new("/tmp/foox")));
        assert!(!rules.matches(Path::new("/tmp/foo")));
        assert!(!rules.matches(Path::new("/tmp/fooxy")));
    }

    #[test]
    fn protected_prefixes_follow_the_root() {
        let under_slash = protected_prefixes_under(Path::new("/"));
        assert!(under_slash.iter().any(|p| p == Path::new("/usr")));
        assert!(under_slash.iter().any(|p| p == Path::new("/System")));
        assert!(protected_prefixes_under(Path::new("/Users/me/Downloads")).is_empty());
        assert!(protected_prefixes_under(Path::new("/usr/local")).is_empty());
        assert!(is_exact_protected_root(Path::new("/usr")));
        assert!(is_exact_protected_root(Path::new("/Library")));
        assert!(!is_exact_protected_root(Path::new("/usr/local")));
        assert!(!is_exact_protected_root(Path::new("/Users/me/Downloads")));
        assert!(!is_exact_protected_root(Path::new("/")));
    }

    #[test]
    fn lexical_normalize_removes_dot_and_dotdot() {
        assert_eq!(
            normalize_lexical(Path::new("/tmp/foo/../bar")),
            Path::new("/tmp/bar")
        );
        assert_eq!(
            normalize_lexical(Path::new("/tmp/./bar")),
            Path::new("/tmp/bar")
        );
    }

    #[test]
    fn empty_exclusion_is_ignored() {
        let rules = set("/tmp/fixture", &[""]);
        assert!(!rules.matches(Path::new("/tmp/fixture/a")));
    }
}
