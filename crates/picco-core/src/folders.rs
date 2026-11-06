//! Which folders are in the library: watched folders with everything inside
//! them, minus excluded ones. The rule closest to a folder wins, so a folder
//! can be watched inside an excluded one, like in Picasa's Folder Manager.

use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FolderState {
    /// Watched itself.
    Watched,
    /// Inside a watched folder.
    InsideWatched,
    /// Excluded itself.
    Excluded,
    /// Inside an excluded folder.
    InsideExcluded,
    /// No rule covers it.
    Outside,
}

impl FolderState {
    pub fn in_library(self) -> bool {
        matches!(self, Self::Watched | Self::InsideWatched)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FolderRules {
    pub watched: Vec<PathBuf>,
    pub excluded: Vec<PathBuf>,
}

impl FolderRules {
    pub fn state(&self, path: &Path) -> FolderState {
        let depth = |rule: &PathBuf| path.starts_with(rule).then(|| rule.components().count());
        let nearest_watch = self.watched.iter().filter_map(depth).max();
        let nearest_exclude = self.excluded.iter().filter_map(depth).max();
        let own = path.components().count();
        match (nearest_watch, nearest_exclude) {
            (Some(w), e) if e.is_none_or(|e| w > e) => {
                if w == own {
                    FolderState::Watched
                } else {
                    FolderState::InsideWatched
                }
            }
            (_, Some(e)) if e == own => FolderState::Excluded,
            (_, Some(_)) => FolderState::InsideExcluded,
            _ => FolderState::Outside,
        }
    }

    /// Puts `path` into the library or takes it out, adding or dropping rules
    /// so that the folder ends up the way asked with as few rules as possible.
    pub fn set_in_library(&mut self, path: &Path, in_library: bool) {
        if self.state(path).in_library() == in_library {
            return;
        }
        if in_library {
            self.excluded.retain(|p| p != path);
            if !self.state(path).in_library() {
                self.watched.push(path.to_path_buf());
            }
        } else {
            self.watched.retain(|p| p != path);
            if self.state(path).in_library() {
                self.excluded.push(path.to_path_buf());
            }
        }
        // An exclusion outside every watched folder does nothing.
        let watched = self.watched.clone();
        self.excluded.retain(|e| watched.iter().any(|w| e.starts_with(w) && e != w));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rules(watched: &[&str], excluded: &[&str]) -> FolderRules {
        FolderRules {
            watched: watched.iter().map(PathBuf::from).collect(),
            excluded: excluded.iter().map(PathBuf::from).collect(),
        }
    }

    #[test]
    fn the_nearest_rule_wins() {
        let r = rules(&["/p", "/p/old/keep"], &["/p/old"]);
        assert_eq!(r.state(Path::new("/p")), FolderState::Watched);
        assert_eq!(r.state(Path::new("/p/2024")), FolderState::InsideWatched);
        assert_eq!(r.state(Path::new("/p/old")), FolderState::Excluded);
        assert_eq!(r.state(Path::new("/p/old/junk")), FolderState::InsideExcluded);
        assert_eq!(r.state(Path::new("/p/old/keep/a")), FolderState::InsideWatched);
        assert_eq!(r.state(Path::new("/pictures")), FolderState::Outside, "a prefix of a name is not a parent");
        assert_eq!(r.state(Path::new("/")), FolderState::Outside);
    }

    #[test]
    fn toggling_keeps_the_rules_minimal() {
        let mut r = rules(&["/p"], &[]);
        r.set_in_library(Path::new("/p/old"), false);
        assert_eq!(r, rules(&["/p"], &["/p/old"]));
        r.set_in_library(Path::new("/p/old/keep"), true);
        assert_eq!(r, rules(&["/p", "/p/old/keep"], &["/p/old"]));
        r.set_in_library(Path::new("/p/old"), true);
        assert_eq!(r, rules(&["/p", "/p/old/keep"], &[]));
        r.set_in_library(Path::new("/p/2024"), true);
        assert_eq!(r, rules(&["/p", "/p/old/keep"], &[]), "already in the library");
        r.set_in_library(Path::new("/p/old"), false);
        r.set_in_library(Path::new("/p"), false);
        assert_eq!(r, rules(&["/p/old/keep"], &[]), "exclusions outside watched folders go away");
    }
}
