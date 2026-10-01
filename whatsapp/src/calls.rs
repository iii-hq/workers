use anyhow::{Result, ensure};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{registry::Registry, validate};

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct CallRequest {
    session_id: String,
    call_id: String,
    peer: String,
    call_creator: String,
}

impl CallRequest {
    fn addresses(&self) -> Result<(whatsapp_rust::Jid, whatsapp_rust::Jid)> {
        validate::text(&self.call_id, "call_id", 256)?;
        let peer = validate::jid(&self.peer)?;
        let creator = validate::jid(&self.call_creator)?;
        ensure!(
            matches!(
                peer.server,
                whatsapp_rust::Server::Pn | whatsapp_rust::Server::Lid
            ) && matches!(
                creator.server,
                whatsapp_rust::Server::Pn | whatsapp_rust::Server::Lid
            ),
            "call peer and creator must be user JIDs"
        );
        Ok((peer, creator))
    }
}

#[derive(Serialize, JsonSchema)]
struct CallResult {
    /// The signaling stanza was sent; no remote acknowledgement is provided.
    requested: bool,
}

pub fn register(registry: &mut Registry) {
    registry.add::<CallRequest, CallResult, _, _>(
        "whatsapp::calls::reject",
        "Request rejection using the call ID, peer and creator from an incoming call event",
        |manager, request| async move {
            let (peer, creator) = request.addresses()?;
            manager
                .client(&request.session_id)
                .await?
                .voip()
                .reject_call(&request.call_id, &peer, &creator)
                .await?;
            Ok(CallResult { requested: true })
        },
    );
    registry.add::<CallRequest, CallResult, _, _>(
        "whatsapp::calls::terminate",
        "Request call termination using the original signaling identifiers",
        |manager, request| async move {
            let (peer, creator) = request.addresses()?;
            manager
                .client(&request.session_id)
                .await?
                .voip()
                .terminate(&request.call_id, &peer, &creator)
                .await?;
            Ok(CallResult { requested: true })
        },
    );
}
