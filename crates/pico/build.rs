//! Sets PICO_VERSION for Help → About: the version in Cargo.toml with its
//! last number raised by the commits made since that version was set, and
//! the commit (0.3.0 and seven commits later: "0.3.7 (c87c8be)"). Without
//! git (a source archive) it is the plain Cargo.toml version.

use std::path::Path;
use std::process::Command;

fn git(args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).output().ok()?;
    let text = String::from_utf8(out.stdout).ok()?.trim().to_owned();
    (out.status.success() && !text.is_empty()).then_some(text)
}

fn main() {
    let version = std::env::var("CARGO_PKG_VERSION").unwrap();
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=../../Cargo.toml");
    println!("cargo:rustc-env=PICO_VERSION={}", described(&version).unwrap_or(version));
}

fn described(version: &str) -> Option<String> {
    // A new commit or branch moves HEAD or the ref it points to.
    let git_dir = git(&["rev-parse", "--git-dir"])?;
    let mut watched = vec!["HEAD".to_owned(), "packed-refs".to_owned()];
    watched.extend(git(&["symbolic-ref", "-q", "HEAD"]));
    for file in watched {
        let path = Path::new(&git_dir).join(file);
        if path.exists() {
            println!("cargo:rerun-if-changed={}", path.display());
        }
    }
    // The commit that last changed the workspace version line.
    let set = git(&["log", "-1", "--format=%H", "-G", "^version = ", "--", ":/Cargo.toml"])?;
    let since: u32 = git(&["rev-list", "--count", &format!("{set}..HEAD")])?.parse().ok()?;
    let head = git(&["rev-parse", "--short", "HEAD"])?;
    let (major_minor, patch) = version.rsplit_once('.')?;
    let patch: u32 = patch.parse().ok()?;
    Some(format!("{major_minor}.{} ({head})", patch + since))
}
