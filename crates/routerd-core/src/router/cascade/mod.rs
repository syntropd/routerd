//! Cascaded routing and elastic model family downgrade subsystem.

pub mod cascade_router;
pub mod elastic_downgrade;

pub use cascade_router::{CascadeDecision, CascadeRouter};
pub use elastic_downgrade::{DowngradeDecision, ElasticFamilyDowngrader, FamilyLadder, VramPressureLevel};
