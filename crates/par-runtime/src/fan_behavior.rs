use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
/// The behavior of a `Package` node when it interacts
/// with a fan node (duplicate or erase)
pub enum FanBehavior {
    /// Expand the package and then duplicate/erase it
    /// Used for side-effectful and top level packages
    Expand,
    /// Propagate the fan operator through the captures
    /// Used in boxes.
    Propagate,
}
