//! `slack::users::*` — directory lookups.

use std::sync::Arc;

use iii_sdk::IIIClient;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::deps::Deps;
use crate::slack_method;

#[derive(Debug, Default, Deserialize, Serialize, JsonSchema)]
pub struct ListReq {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
    #[serde(flatten, default)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct InfoReq {
    pub user: String,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct LookupByEmailReq {
    pub email: String,
}

#[derive(Debug, Default, Deserialize, Serialize, JsonSchema)]
pub struct ProfileGetReq {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
}

slack_method!(
    list,
    "slack::users::list",
    "users.list",
    "List workspace users.",
    ListReq
);
slack_method!(
    info,
    "slack::users::info",
    "users.info",
    "Get a user's profile and metadata.",
    InfoReq
);
slack_method!(
    lookup_by_email,
    "slack::users::lookup-by-email",
    "users.lookupByEmail",
    "Find a user by email address.",
    LookupByEmailReq
);
slack_method!(
    profile_get,
    "slack::users::profile-get",
    "users.profile.get",
    "Get a user's profile fields.",
    ProfileGetReq
);

pub fn register(iii: &Arc<IIIClient>, deps: &Arc<Deps>) {
    list(iii, deps);
    info(iii, deps);
    lookup_by_email(iii, deps);
    profile_get(iii, deps);
}
