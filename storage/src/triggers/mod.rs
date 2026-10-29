//! Object-storage change triggers (`object-created`, `object-deleted`) and
//! their observable functions (`on-object-created`, `on-object-deleted`).

pub mod dispatcher;
pub mod handler;
pub mod normalize;
pub mod object_created;
pub mod object_deleted;
pub mod observe;
pub mod pollers;
pub mod registry;
