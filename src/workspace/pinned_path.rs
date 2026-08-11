use std::path::{Path, PathBuf};

/// Expand a user-entered path into an absolute one. The directory need not
/// exist: a pinned path may point at a repo you have not cloned yet.
pub(crate) fn expand_pinned_path(raw: &str) -> PathBuf {
    let trimmed = raw.trim();
    let expanded = match trimmed.strip_prefix('~') {
        Some(rest) if rest.is_empty() || rest.starts_with('/') => match std::env::var_os("HOME") {
            Some(home) => PathBuf::from(home).join(rest.trim_start_matches('/')),
            None => PathBuf::from(trimmed),
        },
        _ => PathBuf::from(trimmed),
    };
    if expanded.is_absolute() {
        return expanded;
    }
    std::env::current_dir()
        .map(|cwd| cwd.join(&expanded))
        .unwrap_or(expanded)
}

/// Whether `cwd` sits at or below `pinned`. Compared component-wise so
/// `/ws-worktrees` is not treated as living under `/ws`. Canonicalises both
/// sides when they exist so symlinked checkouts match; falls back to a lexical
/// comparison otherwise.
pub(crate) fn path_claims(pinned: &Path, cwd: &Path) -> bool {
    let pinned = std::fs::canonicalize(pinned).unwrap_or_else(|_| pinned.to_path_buf());
    let cwd = std::fs::canonicalize(cwd).unwrap_or_else(|_| cwd.to_path_buf());
    cwd.starts_with(&pinned)
}

/// The pin a workspace should end up with when the user toggles it against
/// `cwd`. `None` clears the pin, which is what a second press at the same
/// directory means. Canonicalises both sides so a pin entered as `~/code/herdr`
/// still toggles off when the shell reports the symlink-resolved path.
pub(crate) fn toggled_pin(current: Option<&Path>, cwd: &Path) -> Option<PathBuf> {
    let already_pinned_here = current.is_some_and(|pinned| {
        crate::worktree::canonical_or_original(pinned)
            == crate::worktree::canonical_or_original(cwd)
    });
    (!already_pinned_here).then(|| cwd.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expands_leading_tilde() {
        let home = std::env::var("HOME").expect("HOME");
        assert_eq!(
            expand_pinned_path("~/code/herdr"),
            PathBuf::from(format!("{home}/code/herdr"))
        );
    }

    #[test]
    fn leaves_absolute_paths_alone() {
        assert_eq!(
            expand_pinned_path("/srv/herdr"),
            PathBuf::from("/srv/herdr")
        );
    }

    #[test]
    fn claims_the_pinned_directory_itself() {
        assert!(path_claims(Path::new("/ws"), Path::new("/ws")));
    }

    #[test]
    fn claims_directories_below_the_pinned_path() {
        assert!(path_claims(Path::new("/ws"), Path::new("/ws/src/app")));
    }

    #[test]
    fn does_not_claim_a_sibling_with_a_shared_prefix() {
        assert!(!path_claims(Path::new("/ws"), Path::new("/ws-worktrees")));
        assert!(!path_claims(Path::new("/ws"), Path::new("/ws-worktrees/a")));
    }

    #[test]
    fn does_not_claim_a_parent_directory() {
        assert!(!path_claims(Path::new("/ws/src"), Path::new("/ws")));
    }

    #[test]
    fn compares_lexically_when_paths_do_not_exist() {
        assert!(path_claims(
            Path::new("/nonexistent-herdr-test/ws"),
            Path::new("/nonexistent-herdr-test/ws/src")
        ));
    }

    #[test]
    fn pins_an_unpinned_workspace_to_the_current_directory() {
        assert_eq!(
            toggled_pin(None, Path::new("/ws")),
            Some(PathBuf::from("/ws"))
        );
    }

    #[test]
    fn clears_a_pin_that_already_matches_the_current_directory() {
        assert_eq!(toggled_pin(Some(Path::new("/ws")), Path::new("/ws")), None);
    }

    #[test]
    fn repins_when_the_current_directory_moved_elsewhere() {
        assert_eq!(
            toggled_pin(Some(Path::new("/ws")), Path::new("/other")),
            Some(PathBuf::from("/other"))
        );
    }

    #[test]
    fn clears_a_pin_that_matches_through_a_symlink() {
        let base = std::env::temp_dir().join(format!("herdr-toggle-pin-{}", std::process::id()));
        let real = base.join("real");
        let link = base.join("link");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&real).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&real, &link).unwrap();
        #[cfg(windows)]
        std::os::windows::fs::symlink_dir(&real, &link).unwrap();

        assert_eq!(toggled_pin(Some(&link), &real), None);

        let _ = std::fs::remove_dir_all(&base);
    }
}
