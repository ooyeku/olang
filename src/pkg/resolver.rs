//! Version resolution across the dependency graph, using Minimal Version
//! Selection (MVS).
//!
//! MVS is the Go-modules algorithm: for each package, select the *lowest*
//! version that satisfies every requirement placed on it across the whole
//! graph. This is deliberately simpler than SemVer-maximal resolution — it is
//! reproducible by construction (no "latest compatible" drift), needs no
//! backtracking SAT solver, and makes upgrades explicit rather than
//! automatic. A requirement is a minimum floor; the selected version is the
//! maximum of all floors, provided one published version satisfies them all.

use crate::pkg::registry::Registry;
use semver::{Version, VersionReq};
use std::collections::BTreeMap;

#[derive(Debug)]
pub enum ResolveError {
    Registry(String),
    NoVersion { name: String, req: String },
    BadReq { name: String, req: String },
    Conflict { name: String, detail: String },
}

impl std::fmt::Display for ResolveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ResolveError::Registry(e) => write!(f, "{}", e),
            ResolveError::NoVersion { name, req } => {
                write!(f, "no published version of '{}' satisfies '{}'", name, req)
            }
            ResolveError::BadReq { name, req } => {
                write!(f, "invalid version requirement '{}' for '{}'", req, name)
            }
            ResolveError::Conflict { name, detail } => {
                write!(f, "version conflict for '{}': {}", name, detail)
            }
        }
    }
}

impl std::error::Error for ResolveError {}

/// The resolved registry graph: package name -> selected version.
pub type Resolution = BTreeMap<String, Version>;

/// Resolve registry dependencies to concrete versions via MVS.
///
/// `roots` are the top-level `(name, version requirement)` pairs from the
/// project manifest. Transitive requirements are read from each release's
/// index entry.
pub fn resolve(
    roots: &[(String, String)],
    registry: &Registry,
) -> Result<Resolution, ResolveError> {
    // For each package, the set of minimum versions any requirement demands.
    // MVS selects the maximum of these minimums.
    let mut selected: Resolution = BTreeMap::new();
    // Work queue of (name, requirement) still to process.
    let mut queue: Vec<(String, String)> = roots.to_vec();

    while let Some((name, req_str)) = queue.pop() {
        let req = VersionReq::parse(&req_str).map_err(|_| ResolveError::BadReq {
            name: name.clone(),
            req: req_str.clone(),
        })?;

        // Candidate versions this requirement admits.
        let available = registry
            .versions(&name)
            .map_err(|e| ResolveError::Registry(e.to_string()))?;
        let mut matching: Vec<Version> = available.into_iter().filter(|v| req.matches(v)).collect();
        matching.sort();

        // MVS: the floor this requirement imposes is the *lowest* matching
        // version.
        let floor = matching
            .first()
            .cloned()
            .ok_or_else(|| ResolveError::NoVersion {
                name: name.clone(),
                req: req_str.clone(),
            })?;

        // Raise the package's selection to the max of all floors seen.
        let current = selected.get(&name).cloned();
        let new_version = match &current {
            Some(existing) if *existing >= floor => existing.clone(),
            _ => floor,
        };

        // If the selection changed (or is new), (re)queue this release's own
        // requirements against the newly selected version.
        let changed = current.as_ref() != Some(&new_version);
        selected.insert(name.clone(), new_version.clone());

        if changed {
            let release = registry
                .release(&name, &new_version)
                .map_err(|e| ResolveError::Registry(e.to_string()))?;
            for (dep_name, dep_req) in release.dependencies {
                queue.push((dep_name, dep_req));
            }
        }
    }

    Ok(selected)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pkg::registry::{Registry, Release};

    fn write_registry(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("olang_reg_{}_{}", std::process::id(), tag));
        let _ = std::fs::remove_dir_all(&dir);
        let reg = Registry::at(&dir);
        // alpha has 1.0.0 (deps beta ^1.0) and 1.1.0 (deps beta ^1.1)
        for (v, beta_req) in [("1.0.0", "1.0.0"), ("1.1.0", "1.1.0")] {
            let mut deps = std::collections::BTreeMap::new();
            deps.insert("beta".to_string(), beta_req.to_string());
            reg.publish(
                "alpha",
                Release {
                    version: Version::parse(v).unwrap(),
                    git: "x".into(),
                    rev: "x".into(),
                    checksum: None,
                    dependencies: deps,
                },
            )
            .unwrap();
        }
        for v in ["1.0.0", "1.1.0", "1.2.0"] {
            reg.publish(
                "beta",
                Release {
                    version: Version::parse(v).unwrap(),
                    git: "x".into(),
                    rev: "x".into(),
                    checksum: None,
                    dependencies: Default::default(),
                },
            )
            .unwrap();
        }
        dir
    }

    #[test]
    fn mvs_picks_lowest_satisfying_version() {
        let dir = write_registry("lowest");
        let reg = Registry::at(&dir);
        // Require alpha ^1.0 -> MVS picks 1.0.0 (lowest matching), which pulls
        // beta ^1.0 -> beta 1.0.0. Not the latest (1.1.0 / 1.2.0).
        let res = resolve(&[("alpha".into(), "^1.0".into())], &reg).unwrap();
        assert_eq!(res["alpha"], Version::new(1, 0, 0));
        assert_eq!(res["beta"], Version::new(1, 0, 0));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn takes_the_max_of_all_floors() {
        let dir = write_registry("maxfloor");
        let reg = Registry::at(&dir);
        // Two requirements on beta: ^1.0 (floor 1.0) and ^1.2 (floor 1.2).
        // MVS selects the max floor, 1.2.0.
        let res = resolve(
            &[
                ("beta".into(), "^1.0".into()),
                ("beta".into(), "^1.2".into()),
            ],
            &reg,
        )
        .unwrap();
        assert_eq!(res["beta"], Version::new(1, 2, 0));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unsatisfiable_requirement_errors() {
        let dir = write_registry("unsat");
        let reg = Registry::at(&dir);
        let err = resolve(&[("beta".into(), "^2.0".into())], &reg).unwrap_err();
        assert!(matches!(err, ResolveError::NoVersion { .. }));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
