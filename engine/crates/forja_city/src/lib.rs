//! # forja_city
//!
//! Procedural Spanish city kit inspired by Valencia and Madrid. Buildings
//! are generated from a lot footprint and a data-driven style
//! (`data/city/styles.toml`): real openings with reveals, mouldings swept
//! along the street front, instanced balcony doors with wrought-iron
//! railings, shutters (alicantinas, louvres, rollers), miradores, shops with
//! roller shutters, arched portals, tiled bands, cornices with brackets and
//! roofs (balustrades with urns, clay tiles with dormers, slate mansards,
//! corner domes and spires). Streets get raised sidewalks with granite
//! curbs, painted markings, Spanish traffic lights and signs, lamps,
//! bollards, containers and trees.
//!
//! Everything is modular: a module is an instanced scene built once per
//! variant, a style is data, and the city layout only hands out lots.
//!
//! Surfaces are weathered so they read as photographed, not generated:
//! scanned materials become larger anti-tiling textures with mottling,
//! repairs, stains, cracks and gum ([`macrotex`]), and every street facade
//! gets dirt decals (rising damp, sill streaks, cornice soot,
//! [`weathering`]).

pub mod building;
pub mod furniture;
pub mod layout;
pub mod geom;
pub mod kit;
pub mod macrotex;
pub mod modules;
pub mod noise;
pub mod style;
pub mod texgen;
pub mod weathering;

pub use building::{CityOut, EdgeKind, Frontage, Lot, ModuleLib};
pub use kit::{Kit, Mat};
pub use style::{Style, StyleLibrary};
