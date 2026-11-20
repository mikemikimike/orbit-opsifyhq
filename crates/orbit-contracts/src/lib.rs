//! Contracts shared across the control plane, workers and harness adapters.
//!
//! These types are deliberately independent of HTTP and SQL. Adapters implement the
//! traits here; the control plane only ever sees these structures.

pub mod connector;
pub mod events;
pub mod harness;
pub mod portable;
pub mod provider;

pub use connector::*;
pub use events::*;
pub use harness::*;
pub use provider::*;
