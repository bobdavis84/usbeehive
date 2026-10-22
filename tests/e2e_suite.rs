#![cfg(feature = "cli")]

//! Opaque-box E2E test suite for usbeehive across Requirements R1 through R7.
//!
//! Tiers 1-4 cover:
//! - Tier 1: Feature Coverage (Happy Path, >=5 per area R1-R7)
//! - Tier 2: Boundary & Corner Cases (>=5 per area R1-R7)
//! - Tier 3: Cross-Feature Combinations (Pairwise)
//! - Tier 4: Real-World Multi-Device Topologies

#[path = "fixture_builder.rs"]
mod fixture_builder;

mod e2e;
