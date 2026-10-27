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
//!
//! ## Performance
//!
//! Fixed step of 1/20 s ([`SimConfig::dt`]), one thread, spatial hashing
//! for vehicles and pedestrians. Measured in release on a shared 4-CPU
//! container (other builds running, so these are upper bounds):
//!
//! | scene | agents | mean step | per 60 Hz frame |
//! |---|---|---|---|
//! | headless grid test, 5 min | 93 vehicles, 190 peds | 240 us | ~80 us |
//! | full Spanish city (`city_review`), 8 min | 100 vehicles, 338 peds | 812 us (vehicles 357, peds 265) | ~270 us |
//! | sandbox budget (70 vehicles, 56 peds near the player) | | | `sim 0.37 ms/frame` for the whole game |
//!
//! All well inside the 2 ms budget. The one long step is the first, which
//! populates the whole spawn area at once (0.57 s for 338 pedestrians with
//! their routes on the full map); after it, max steps stay at a few ms.

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
