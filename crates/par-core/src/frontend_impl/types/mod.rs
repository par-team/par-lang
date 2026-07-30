pub(crate) mod core;
pub(crate) use core::LoopId;
pub use core::{Operation, PrimitiveType, Type};
pub(crate) use core::{TypePath, TypePathSegment};

pub(crate) mod visibility;
pub(crate) use visibility::{Visibility, VisibilityIndex, validate_visibility};

pub(crate) mod error;
pub use error::TypeError;

pub(crate) mod definitions;
pub use definitions::TypeDefs;
pub(crate) mod assignability;
pub(crate) use assignability::{SubtypeMismatchCause, SubtypeMismatchKind};
pub(crate) mod checking;
pub(crate) mod context;
pub(crate) use context::Context;
pub(crate) mod dependencies;
pub(crate) mod display;
pub use display::GlobalNameWriter;
pub(crate) mod duality;
pub(crate) mod expansion;
mod implicit;
pub(crate) mod lattice;
pub(crate) mod registry;
pub(crate) mod substitution;
pub(crate) mod tests;
pub(crate) mod validation;
pub(crate) mod visit;
