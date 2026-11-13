//! Drive REST calls, one module per owner (ARCHITECTURE §2.3).
//! Module declarations are owned by W1.1; each file is owned by the unit named in its header.

pub mod client;
pub mod home;
pub mod manage;
pub mod perms;
pub mod relay;
pub mod revisions;
