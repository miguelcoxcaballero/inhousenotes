//! # forja_ai
//!
//! Traffic and pedestrian AI that obeys the signs and lights of the city.
//!
//! - [`spec`]: the abstract road description ([`RoadSource`] /
//!   [`RoadGraphSpec`]) any street generator can produce, and a synthetic
//!   grid city for tests;
//! - [`adapter`]: street-segment soup + placed signs -> road graph (used
//!   for `forja_city`'s current layout, see the `city` feature);
//! - [`network`]: lanes, turn connectors and their conflict zones, stop
//!   lines, crosswalks, signal controllers, the pedestrian graph and A*;
//! - [`signals`]: fixed-cycle controllers with offsets and pedestrian
//!   heads; [`network::HeadBinding`] ties every visible head to the state
//!   the drivers obey;
//! - [`driver`]: IDM, amber dilemma, MOBIL, pure pursuit;
//! - [`sim`]: the deterministic fixed-step world (spawning budgets, LOD,
//!   right of way, gap acceptance, crosswalk yielding, collisions stats);
//! - [`peds`]: pedestrians (kerb waiting, crossing, activities, fleeing,
//!   social forces, animation selection);
//! - [`debug`]: a top-down raster view for review.

pub mod adapter;
#[cfg(feature = "city")]
pub mod city;
pub mod debug;
pub mod driver;
pub mod geom;
pub mod network;
pub mod peds;
pub mod rng;
pub mod signals;
pub mod sim;
pub mod spatial;
pub mod spec;

pub use network::{HeadBinding, RoadNetwork};
pub use peds::{Activity, Ped, PedAnim, PedState};
pub use signals::{Aspect, PedSignal};
pub use sim::{External, ExternalKind, ParkedCar, SimConfig, SimEvent, SimStats, TrafficSim, Vehicle};
pub use spec::{Control, RoadGraphSpec, RoadSource};
