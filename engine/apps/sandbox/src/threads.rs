//! Worker threads that run while the game plays must never take CPU time
//! from the main thread: see [`forja_assets::threads`].

pub use forja_assets::threads::*;
