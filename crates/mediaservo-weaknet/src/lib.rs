//! mediaservo-weaknet thin lib = the unit-test reachable surface (spec/engine/scope/state/fuse modules arrive with T2+;
//! config/watch arrive with the T12 vehicle surface).
//! **This lib is not in the bindings / package manifest** (W5 three-predicate audit); zero product coupling.

pub mod config;
pub mod engine;
pub mod measure;
pub mod fuse;
pub mod ifb;
pub mod scenario;
pub mod scope;
pub mod server;
pub mod spec;
pub mod state;
pub mod watch;
