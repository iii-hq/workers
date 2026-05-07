//! Object-storage change triggers (`object-created`, `object-deleted`).

pub mod dispatcher;
pub mod handler;
pub mod normalize;
pub mod object_created;
pub mod object_deleted;
pub mod pollers;
pub mod registry;
