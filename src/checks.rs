//! Checks encapsulate a multitude of checks that can be performed on the cloud
//! provider.
//!
//! Right now the following checks are implemented:
//! - network: can check basic subnet configuration (number of subnets, tags).
//!
//! Planned checks:
//! - Compare LB setup to configured subnets.

pub mod network;
