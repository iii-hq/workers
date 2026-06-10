use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChannelDirection {
    Read,
    Write,
}

/// Wire shape of an iii streaming-channel endpoint (a forwardable bearer
/// token): `channel_id` + `access_key` are minted by the engine's channel
/// primitive and grant one direction of access.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StreamChannelRef {
    pub channel_id: String,
    pub access_key: String,
    pub direction: ChannelDirection,
}
