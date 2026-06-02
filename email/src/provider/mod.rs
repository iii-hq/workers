pub mod imap;
pub mod smtp;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Wire-identical mirror of `iii_sdk::channels::StreamChannelRef`. The SDK
/// type does not derive `JsonSchema`, which would block typed registration
/// of `email::search` and `email::attachment::get`. Converted via `From` at
/// the handler boundary so the rest of the worker keeps using the SDK type.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
pub struct StreamRef {
    pub channel_id: String,
    pub access_key: String,
    #[serde(default)]
    pub direction: StreamDirection,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "lowercase")]
pub enum StreamDirection {
    Read,
    #[default]
    Write,
}

impl From<StreamRef> for iii_sdk::channels::StreamChannelRef {
    fn from(s: StreamRef) -> Self {
        iii_sdk::channels::StreamChannelRef {
            channel_id: s.channel_id,
            access_key: s.access_key,
            direction: match s.direction {
                StreamDirection::Read => iii_sdk::channels::ChannelDirection::Read,
                StreamDirection::Write => iii_sdk::channels::ChannelDirection::Write,
            },
        }
    }
}
