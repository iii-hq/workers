//! Requests in flight between the server and `http::on-request`'s deliver
//! hook.
//!
//! The server registers each request before it calls `http::on-request`, and
//! names it in the call's metadata under [`REQUEST_ID_KEY`] (the function is
//! private, so only the server makes these calls). The hook reads from it what
//! the call's input does not carry: which listener received the request and
//! the request's exact bytes. When a route's target is about to run, the hook
//! opens the request-body and response channels and hands the response
//! channel's reader to the server, which streams what the target writes to
//! the client. The hook also records the matched route's template, which
//! labels the request's span and metrics.

use std::sync::{Arc, Mutex, OnceLock};

use axum::body::Bytes;
use dashmap::DashMap;
use iii_sdk::channel::ChannelReader;
use tokio::sync::oneshot;

/// Key of the call metadata that names the request in flight.
pub const REQUEST_ID_KEY: &str = "request_id";

/// One request in flight.
pub struct InFlight {
    public_webhooks_only: bool,
    body: Option<Bytes>,
    route: OnceLock<String>,
    response: Mutex<Option<oneshot::Sender<ChannelReader>>>,
}

impl InFlight {
    /// Whether the restricted listener received the request.
    pub fn public_webhooks_only(&self) -> bool {
        self.public_webhooks_only
    }

    /// The request's exact bytes; `None` when the body is over the server's
    /// limit.
    pub fn body(&self) -> Option<&Bytes> {
        self.body.as_ref()
    }

    /// Records the template of the route the request matched.
    pub fn set_route(&self, template: &str) {
        let _ = self.route.set(template.to_string());
    }

    /// The template of the route the request matched, once the hook matched
    /// one.
    pub fn route(&self) -> Option<&str> {
        self.route.get().map(String::as_str)
    }

    /// Hands the server the response channel's reader. `false` when it was
    /// handed over already or the server no longer waits for it.
    pub fn hand_over(&self, reader: ChannelReader) -> bool {
        let sender = self
            .response
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take();
        sender.is_some_and(|sender| sender.send(reader).is_ok())
    }
}

/// The requests in flight, by id. Shared by the server and the hook.
#[derive(Clone, Default)]
pub struct InFlightRequests(Arc<DashMap<String, Arc<InFlight>>>);

impl InFlightRequests {
    /// Registers a request. It stays in flight until the returned [`Ticket`]
    /// drops; the receiver yields the response channel's reader once the hook
    /// opened the channels.
    pub fn open(
        &self,
        public_webhooks_only: bool,
        body: Option<Bytes>,
    ) -> (Ticket, oneshot::Receiver<ChannelReader>) {
        let (sender, receiver) = oneshot::channel();
        let request = Arc::new(InFlight {
            public_webhooks_only,
            body,
            route: OnceLock::new(),
            response: Mutex::new(Some(sender)),
        });
        let id = uuid::Uuid::new_v4().to_string();
        self.0.insert(id.clone(), request.clone());
        let ticket = Ticket {
            id,
            request,
            requests: self.clone(),
        };
        (ticket, receiver)
    }

    /// The request in flight under `id`.
    pub fn get(&self, id: &str) -> Option<Arc<InFlight>> {
        self.0.get(id).map(|entry| entry.value().clone())
    }
}

/// The server's hold on a request in flight; dropping it removes the request.
pub struct Ticket {
    id: String,
    request: Arc<InFlight>,
    requests: InFlightRequests,
}

impl Ticket {
    /// The id the server names in the call's metadata.
    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn request(&self) -> &InFlight {
        &self.request
    }
}

impl Drop for Ticket {
    fn drop(&mut self) {
        self.requests.0.remove(&self.id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use iii_sdk::channel::StreamChannelRef;

    fn reader() -> ChannelReader {
        let channel: StreamChannelRef = serde_json::from_value(serde_json::json!({
            "channel_id": "c", "access_key": "k", "direction": "read"
        }))
        .unwrap();
        ChannelReader::new("ws://127.0.0.1:9", &channel)
    }

    #[test]
    fn a_request_is_in_flight_until_its_ticket_drops() {
        let requests = InFlightRequests::default();
        let (ticket, _receiver) = requests.open(true, Some(Bytes::from_static(b"raw")));
        let id = ticket.id().to_string();
        let request = requests.get(&id).expect("in flight");
        assert!(request.public_webhooks_only());
        assert_eq!(request.body().map(|b| b.as_ref()), Some(&b"raw"[..]));
        drop(ticket);
        assert!(requests.get(&id).is_none());
    }

    #[test]
    fn the_hook_records_the_matched_route_once() {
        let requests = InFlightRequests::default();
        let (ticket, _receiver) = requests.open(false, None);
        assert_eq!(ticket.request().route(), None);
        let request = requests.get(ticket.id()).unwrap();
        request.set_route("/orders/:id");
        request.set_route("/other");
        assert_eq!(ticket.request().route(), Some("/orders/:id"));
        assert!(ticket.request().body().is_none());
    }

    #[tokio::test]
    async fn the_response_reader_is_handed_over_once() {
        let requests = InFlightRequests::default();
        let (ticket, receiver) = requests.open(false, Some(Bytes::new()));
        let request = requests.get(ticket.id()).unwrap();
        assert!(request.hand_over(reader()));
        assert!(!request.hand_over(reader()), "a second reader is refused");
        assert!(receiver.await.is_ok());

        let (ticket, receiver) = requests.open(false, Some(Bytes::new()));
        drop(receiver);
        assert!(
            !requests.get(ticket.id()).unwrap().hand_over(reader()),
            "nobody waits for it"
        );
    }
}
