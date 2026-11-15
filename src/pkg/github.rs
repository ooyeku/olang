//! Libraries from GitHub (OL-582): `owner/repo[@ref]` fetched with the
//! person's own `git`, checked to be an olang project, and held on the
//! shelf as a copy with its commit and checksum.
//!
//! Nothing here runs a library's code: a fetch is `git` and `tar`, and the
//! only file read is its `olang.toml`. `OLANG_GITHUB` says where GitHub is
//! (default `https://github.com`); the test suite points it at a local
//! fixture.

use super::shelf::{Shelf, ShelfCopy, ShelfError};
use std::path::PathBuf;

/// Where GitHub is: `$OLANG_GITHUB`, else `https://github.com`.
pub fn base() -> String {
    std::env::var("OLANG_GITHUB")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "https://github.com".to_string())
        .trim_end_matches('/')
        .to_string()
}

/// The git URL of `owner/repo`.
pub fn repo_url(owner_repo: &str) -> String {
    format!("{}/{}.git", base(), owner_repo)
}

fn plain(part: &str) -> bool {
    !part.is_empty()
        && !part.starts_with(['-', '.'])
        && part
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.')
}

/// `owner/repo`, `owner/repo@ref`, `github.com/owner/repo`, or
/// `https://github.com/owner/repo(.git)` read as `(owner/repo, ref)`.
/// `None` for anything else.
pub fn parse_spec(spec: &str) -> Option<(String, Option<String>)> {
    let s = spec.trim();
    let (s, reference) = match s.rsplit_once('@') {
        Some((a, r)) if !r.is_empty() && !r.contains('/') => (a, Some(r.to_string())),
        _ => (s, None),
    };
    let s = s
        .strip_prefix("https://")
        .or_else(|| s.strip_prefix("http://"))
        .unwrap_or(s);
    let s = s.strip_prefix("github.com/").unwrap_or(s);
    let s = s.trim_end_matches('/').trim_end_matches(".git");
    let mut parts = s.split('/');
    let (Some(owner), Some(repo), None) = (parts.next(), parts.next(), parts.next()) else {
        return None;
    };
    if !plain(owner) || !plain(repo) {
        return None;
    }
    if reference.as_deref().is_some_and(|r| r.starts_with('-')) {
        return None;
    }
    Some((format!("{owner}/{repo}"), reference))
}

/// What a dependency or a command asks of a repository.
#[derive(Debug, Clone, PartialEq)]
pub enum Ask {
    /// The newest release tag, else the default branch's head.
    Newest,
    /// The newest release tag whose version the requirement allows.
    Version(String),
    Tag(String),
    Branch(String),
    Rev(String),
}

impl Ask {
    /// A `@ref` as typed: a version-like tag, a commit id, or a branch.
    /// A ref the remote has as a tag is a tag (decided when fetched).
    pub fn from_ref(r: Option<&str>) -> Ask {
        match r {
            None | Some("") => Ask::Newest,
            Some(r) if r.len() >= 7 && r.bytes().all(|b| b.is_ascii_hexdigit()) => {
                Ask::Rev(r.to_string())
            }
            Some(r) => Ask::Tag(r.to_string()),
        }
    }
}

/// A release tag's version: `v1.2.0` and `1.2.0` are 1.2.0.
pub fn tag_version(tag: &str) -> Option<semver::Version> {
    semver::Version::parse(tag.trim_start_matches('v')).ok()
}

/// The remote's tags and branches, as `(name, commit)`.
pub struct Remote {
    pub tags: Vec<(String, String)>,
    pub branches: Vec<(String, String)>,
    pub head: Option<(String, String)>,
}

/// Read the remote's refs (network).
pub fn remote(owner_repo: &str) -> Result<Remote, ShelfError> {
    let url = repo_url(owner_repo);
    let refs = super::cache::remote_refs(&url).map_err(|e| {
        ShelfError::Io(format!(
            "could not reach {owner_repo} ({url}): {e}. A private repository is read with your \
             own git credentials: check that `git ls-remote {url}` works"
        ))
    })?;
    let mut tags = Vec::new();
    let mut branches = Vec::new();
    let mut head_sha = None;
    for (name, sha) in refs {
        if let Some(t) = name.strip_prefix("refs/tags/") {
            tags.push((t.to_string(), sha));
        } else if let Some(b) = name.strip_prefix("refs/heads/") {
            branches.push((b.to_string(), sha));
        } else if name == "HEAD" {
            head_sha = Some(sha);
        }
    }
    let head = head_sha.map(|sha| {
        let name = branches
            .iter()
            .find(|(b, s)| s == &sha && (b == "main" || b == "master"))
            .or_else(|| branches.iter().find(|(_, s)| s == &sha))
            .map(|(b, _)| b.clone())
            .unwrap_or_else(|| "HEAD".to_string());
        (name, sha)
    });
    Ok(Remote {
        tags,
        branches,
        head,
    })
}

impl Remote {
    /// The release tags, newest first (stable ones before prereleases of
    /// the same rank is semver's order already).
    pub fn releases(&self) -> Vec<(String, semver::Version, String)> {
        let mut out: Vec<(String, semver::Version, String)> = self
            .tags
            .iter()
            .filter_map(|(t, c)| tag_version(t).map(|v| (t.clone(), v, c.clone())))
            .collect();
        out.sort_by(|a, b| b.1.cmp(&a.1));
        out
    }

    /// The newest release tag, stable preferred.
    pub fn newest_release(&self) -> Option<(String, semver::Version, String)> {
        let all = self.releases();
        all.iter()
            .find(|(_, v, _)| v.pre.is_empty())
            .or_else(|| all.first())
            .cloned()
    }

    /// What `ask` names here: `(ref, commit, is_tag)`.
    pub fn resolve(
        &self,
        owner_repo: &str,
        ask: &Ask,
    ) -> Result<(String, String, bool), ShelfError> {
        match ask {
            Ask::Newest => {
                if let Some((t, _, c)) = self.newest_release() {
                    return Ok((t, c, true));
                }
                self.head.clone().map(|(b, c)| (b, c, false)).ok_or_else(|| {
                    ShelfError::NotALibrary(format!("{owner_repo} has no release tag and no default branch"))
                })
            }
            Ask::Version(req) => self
                .releases()
                .into_iter()
                .find(|(_, v, _)| super::shelf::version_matches(req, &v.to_string()))
                .map(|(t, _, c)| (t, c, true))
                .ok_or_else(|| {
                    ShelfError::NotALibrary(format!(
                        "{owner_repo} has no release tag that version {req} allows (its newest tags: {})",
                        self.releases()
                            .iter()
                            .map(|(t, _, _)| t.as_str())
                            .take(8)
                            .collect::<Vec<_>>()
                            .join(", ")
                    ))
                }),
            Ask::Tag(t) => {
                if let Some((_, c)) = self.tags.iter().find(|(n, _)| n == t) {
                    return Ok((t.clone(), c.clone(), true));
                }
                if let Some((_, c)) = self.branches.iter().find(|(n, _)| n == t) {
                    return Ok((t.clone(), c.clone(), false));
                }
                Err(ShelfError::NotALibrary(format!(
                    "{owner_repo} has no tag or branch '{t}'"
                )))
            }
            Ask::Branch(b) => self
                .branches
                .iter()
                .find(|(n, _)| n == b)
                .map(|(n, c)| (n.clone(), c.clone(), false))
                .ok_or_else(|| ShelfError::NotALibrary(format!("{owner_repo} has no branch '{b}'"))),
            Ask::Rev(r) => Ok((r.clone(), r.clone(), false)),
        }
    }
}

/// What a fetch did.
pub struct Fetched {
    pub name: String,
    pub copy: ShelfCopy,
    /// Whether the copy was already held (nothing fetched).
    pub held: bool,
}

/// Fetch `owner_repo` as `ask` asks into the shelf (network), unless a
/// copy of that commit is held already. The tree must be an olang
/// library, or it is refused and nothing is kept. A tag the shelf holds
/// at another commit is refused unless `accept_moved`. `name` overrides
/// the library's own name. The shelf is changed in memory; the caller
/// saves it.
pub fn fetch(
    shelf: &mut Shelf,
    owner_repo: &str,
    ask: &Ask,
    name: Option<&str>,
    accept_moved: bool,
) -> Result<Fetched, ShelfError> {
    let url = repo_url(owner_repo);
    let (reference, commit, is_tag) = match ask {
        // a full commit needs no listing
        Ask::Rev(r) if r.len() == 40 => (r.clone(), r.clone(), false),
        _ => remote(owner_repo)?.resolve(owner_repo, ask)?,
    };
    // held already: by commit, among this repository's copies
    for (lib_name, lib) in &shelf.libraries {
        if name.is_some_and(|n| n != lib_name) {
            continue;
        }
        if let Some(c) = lib.copies.iter().find(|c| {
            super::shelf::same_origin(c.from.as_deref(), owner_repo)
                && c.commit
                    .as_deref()
                    .is_some_and(|h| h.starts_with(&commit) || commit.starts_with(h))
        }) {
            return Ok(Fetched {
                name: lib_name.clone(),
                copy: c.clone(),
                held: true,
            });
        }
        if is_tag
            && let Some(c) = lib.by_ref(Some(owner_repo), &reference)
            && c.commit.as_deref() != Some(commit.as_str())
            && !accept_moved
        {
            return Err(ShelfError::NotALibrary(format!(
                "tag {reference} of {owner_repo} has moved: the shelf holds it at {}, GitHub now has it at {}. \
                 Nothing was changed; accept the new commit with `otc shelf add {owner_repo}@{reference} --accept-moved`",
                c.short_commit().unwrap_or_default(),
                &commit[..commit.len().min(12)]
            )));
        }
    }
    let fetched = super::cache::fetch_git(&url, &super::cache::GitRef::Rev(commit.clone()), false)
        .map_err(|e| {
            ShelfError::Io(format!(
                "could not fetch {owner_repo} at {reference}: {e}. A private repository is read with \
                 your own git credentials: check that `git clone {url}` works"
            ))
        })?;
    let commit = fetched.rev.clone().unwrap_or(commit);
    if let Err(e) = super::shelf::check_library(&fetched.dir) {
        super::cache::forget(&url, Some(&commit));
        return Err(ShelfError::NotALibrary(format!(
            "{owner_repo} was refused: {e}; nothing was kept"
        )));
    }
    let manifest = super::manifest::Manifest::load(&fetched.dir).ok();
    let lib_name = name
        .map(str::to_string)
        .or_else(|| manifest.as_ref().map(|m| m.package.name.clone()))
        .unwrap_or_else(|| {
            owner_repo
                .rsplit('/')
                .next()
                .unwrap_or(owner_repo)
                .to_string()
        });
    let version = manifest
        .as_ref()
        .map(|m| m.package.version.to_string())
        .or_else(|| {
            is_tag
                .then(|| tag_version(&reference).map(|v| v.to_string()))
                .flatten()
        })
        .unwrap_or_else(|| "0.0.0".to_string());
    let copy = shelf.add_copy(
        &lib_name,
        &fetched.dir,
        ShelfCopy {
            version,
            source: "github".to_string(),
            from: Some(owner_repo.to_string()),
            reference: Some(reference),
            commit: Some(commit),
            checksum: None,
            dir: String::new(),
            fetched: None,
        },
    )?;
    Ok(Fetched {
        name: lib_name,
        copy,
        held: false,
    })
}

/// The copy of `owner_repo` at commit `rev` for library `name`: held, or
/// fetched (network) when `may_fetch`. The shelf is saved when a copy is
/// made.
pub fn copy_at(
    shelf: &mut Shelf,
    name: &str,
    owner_repo: &str,
    rev: &str,
    may_fetch: bool,
) -> Result<Option<PathBuf>, ShelfError> {
    if let Some(dir) = shelf.held_copy(name, rev) {
        return Ok(Some(dir));
    }
    if !may_fetch {
        return Ok(None);
    }
    let f = fetch(
        shelf,
        owner_repo,
        &Ask::Rev(rev.to_string()),
        Some(name),
        false,
    )?;
    shelf.save()?;
    Ok(Some(f.copy.path()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn specs_read_as_owner_repo_and_ref() {
        assert_eq!(
            parse_spec("ooyeku/geometry"),
            Some(("ooyeku/geometry".into(), None))
        );
        assert_eq!(
            parse_spec("ooyeku/geometry@v1.2.0"),
            Some(("ooyeku/geometry".into(), Some("v1.2.0".into())))
        );
        assert_eq!(
            parse_spec("https://github.com/ooyeku/geometry.git"),
            Some(("ooyeku/geometry".into(), None))
        );
        assert_eq!(
            parse_spec("github.com/ooyeku/geometry@main"),
            Some(("ooyeku/geometry".into(), Some("main".into())))
        );
        assert_eq!(parse_spec("../geometry"), None);
        assert_eq!(parse_spec("a/b/c"), None);
        assert_eq!(parse_spec("geometry"), None);
        assert_eq!(parse_spec("-x/y"), None);
        assert_eq!(parse_spec("o/r@-upload"), None);
        assert_eq!(Ask::from_ref(Some("9b03e7d5")), Ask::Rev("9b03e7d5".into()));
        assert_eq!(Ask::from_ref(Some("main")), Ask::Tag("main".into()));
    }
}
