//! learn — machine learning over Frames: preprocessing, linear models,
//! trees and ensembles, neighbours, naive Bayes, clustering, PCA and the
//! metrics to judge them.
//!
//! Pure Rust with no dependencies beyond the crate's own (rayon for the
//! parallel paths, statrs for p-values), so the browser build carries
//! it. Every kernel that fans out takes the caller's `par` flag and has
//! a sequential twin with identical results; every random choice comes
//! from a seeded PCG stream ([`rng::Pcg`]), one stream per independent
//! piece of work (a tree, a start, a repeat), so results do not depend
//! on thread count or scheduling.
//!
//! The olang binding is `src/ods/learn.rs` (the `learn` module) and the
//! keyword layer `ml.ol` (`use ml`).

// The kernels index several parallel arrays (rows, classes, features) by
// one position; an iterator chain per array would hide that, not help it.
#![allow(clippy::needless_range_loop)]

pub mod boost;
pub mod cluster;
pub mod data;
pub mod forest;
pub mod formula;
pub mod linalg;
pub mod linear;
pub mod metrics;
pub mod model;
pub mod neighbors;
pub mod preprocess;
pub mod rng;
pub mod tree;

pub use data::{Design, Labels, Matrix};
pub use model::{
    Algo, Body, ClassWeight, Common, MaxFeatures, Model, Output, PredictKind, Task, TaskHint,
    TreeOpts,
};
pub use rng::Pcg;
