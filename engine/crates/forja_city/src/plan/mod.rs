//! The city plan in 2D: the region kernel, polylines, the draft every
//! generator writes and the planar street graph. Nothing here meshes
//! anything.

pub mod draft;
pub mod graph;
pub mod index;
pub mod line;
pub mod region;
pub mod relief;

pub use draft::{Draft, Footprint, Mask, NodeMark, Plaza, PlazaKind, Way};
pub use index::{Located, SegIndex};
pub use graph::{End, Node, NodeId, Street, StreetGraph, StreetId};
pub use line::{Polyline, noise1, noise2};
pub use region::{Join, Region, Shape, Tiled};
pub use relief::{Hill, Relief, STAIR_GRADE, Steps};
