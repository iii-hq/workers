//! Object change events: `storage::on-object-created` and
//! `storage::on-object-deleted`, the private functions a notifications
//! source (an S3/SQS, GCS/Pub/Sub or R2/Cloudflare Queues poller, or the
//! local backend) calls once per object event, and the hooks that deliver
//! each call to the observers of its bucket.

pub mod dispatcher;
pub mod normalize;
pub mod object_created;
pub mod object_deleted;
pub mod pollers;
pub mod source;
