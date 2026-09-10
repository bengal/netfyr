//! Core data model for netfyr network device state.
//!
//! This crate is a **pure representation** layer: it defines the types
//! every other netfyr crate builds on. It deliberately does not:
//!
//! - merge or deduplicate contributions to the same device (that is the
//!   reconciliation layer, which alone can resolve which contributions
//!   target the same live device);
//! - query or depend on the live system;
//! - decode or encode YAML, validate schemas, or infer field types (those are
//!   the schema-directed codec's job).
//!
//! A [`State`] is one source's contribution to one device: a device type
//! (`State::device_type`), a selector identifying the target
//! (`State::match_spec`), the configuration fields being contributed
//! (`State::fields`), plus runtime-only bookkeeping (`State::source`,
//! `State::priority`, `State::metadata`). The same type is used for
//! *desired* state (contributions from policies or dynamic providers,
//! typically a partial match and empty device type) and for *actual* state
//! (queried from the kernel, a fully populated match and device type).
//!
//! Configuration is represented as a plain ordered `Vec<State>`:
//! contributions coexist, are never merged or rejected here, and their list
//! order is significant (the kernel uses the first address on an interface
//! as the primary source address).
//!
pub mod match_spec;
pub mod source;
pub mod state;
pub mod value;

pub use match_spec::Match;
pub use source::Source;
pub use state::{State, StateMetadata};
pub use value::Value;
