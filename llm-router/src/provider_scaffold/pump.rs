//! Forward upstream events into the router-owned channel (spec § Provider
//! stream contract): AssistantMessageEvent frames as JSON text messages,
//! terminal done/error last, pings through silence. Previously a verbatim
//! copy in every provider crate.
use crate::chat::relay::FrameSink;
use crate::types::events::AssistantMessageEvent;
use std::time::Duration;
use tokio::sync::mpsc;

/// Heartbeat cadence while the upstream is silent (spec: at least every 30s).
pub const PING_INTERVAL: Duration = Duration::from_secs(30);

/// `Err(())` means only "the sink is gone — stop writing"; there is no
/// error detail to carry, so the unit error is the whole contract.
#[allow(clippy::result_unit_err)]
pub fn send_event(sink: &dyn FrameSink, ev: &AssistantMessageEvent) -> Result<(), ()> {
    let frame = serde_json::to_string(ev).expect("serializable event");
    sink.send(&frame).map_err(|_| ())
}

/// Forward upstream events to the sink; ping through silence; stop on the
/// terminal event or on a failed write (caller gone → dropping `rx` aborts
/// the upstream task and its in-flight HTTP request).
pub async fn pump(
    mut rx: mpsc::Receiver<AssistantMessageEvent>,
    sink: &dyn FrameSink,
    ping_interval: Duration,
) {
    loop {
        match tokio::time::timeout(ping_interval, rx.recv()).await {
            Ok(Some(ev)) => {
                let terminal = ev.is_terminal();
                if send_event(sink, &ev).is_err() {
                    return;
                }
                if terminal {
                    return;
                }
            }
            // Upstream task ended without a terminal (panic/abort): the
            // router synthesizes the terminal frame — never two terminals.
            Ok(None) => return,
            // Silent stretch: heartbeat (also probes for a gone caller).
            Err(_elapsed) => {
                if send_event(sink, &AssistantMessageEvent::Ping).is_err() {
                    return;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::relay::{ReadEvent, RelayRead};
    use crate::chat::synthesize::empty_partial;
    use crate::testkit::fake_channels::FakeChannel;
    use crate::types::messages::AssistantMessage;
    use serde_json::Value;

    fn now_ms() -> i64 {
        use std::time::{SystemTime, UNIX_EPOCH};
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0)
    }

    fn empty_assistant(model: &str) -> AssistantMessage {
        empty_partial(model, "test-provider", now_ms())
    }

    fn done_event() -> AssistantMessageEvent {
        AssistantMessageEvent::Done {
            message: empty_assistant("model-test"),
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn forwards_events_and_stops_at_terminal() {
        let ch = FakeChannel::new();
        let (tx, rx) = mpsc::channel(8);
        tx.send(AssistantMessageEvent::Start {
            partial: empty_assistant("m"),
        })
        .await
        .unwrap();
        tx.send(done_event()).await.unwrap();
        // a frame after the terminal must never be forwarded
        tx.send(AssistantMessageEvent::Ping).await.unwrap();
        drop(tx);

        pump(rx, &ch.writer, Duration::from_secs(30)).await;
        ch.writer.close();

        let mut frames = Vec::new();
        let mut reader = ch.reader;
        while let ReadEvent::Msg(m) = reader.next(Duration::from_millis(100)).await {
            frames.push(m);
        }
        assert_eq!(frames.len(), 2);
        let last: Value = serde_json::from_str(&frames[1]).unwrap();
        assert_eq!(last["type"], "done");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn pings_through_silence() {
        let ch = FakeChannel::new();
        let (tx, rx) = mpsc::channel::<AssistantMessageEvent>(8);
        // hold tx open, send nothing for > 2 ping intervals, then terminate
        let pump_task = {
            let writer = ch.writer.clone();
            tokio::spawn(async move { pump(rx, &writer, Duration::from_millis(50)).await })
        };
        tokio::time::sleep(Duration::from_millis(140)).await;
        tx.send(done_event()).await.unwrap();
        drop(tx);
        pump_task.await.unwrap();
        ch.writer.close();

        let mut frames = Vec::new();
        let mut reader = ch.reader;
        while let ReadEvent::Msg(m) = reader.next(Duration::from_millis(100)).await {
            frames.push(m);
        }
        let pings = frames
            .iter()
            .filter(|f| serde_json::from_str::<Value>(f).unwrap()["type"] == "ping")
            .count();
        assert!(
            pings >= 2,
            "want >=2 pings through 140ms of silence, got {pings}"
        );
        assert_eq!(
            serde_json::from_str::<Value>(frames.last().unwrap()).unwrap()["type"],
            "done"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn reader_close_stops_the_pump_and_drops_the_receiver() {
        let ch = FakeChannel::new();
        ch.reader.close(); // caller gone before anything is written
        let (tx, rx) = mpsc::channel(8);
        tx.send(AssistantMessageEvent::Start {
            partial: empty_assistant("m"),
        })
        .await
        .unwrap();
        pump(rx, &ch.writer, Duration::from_secs(30)).await; // returns immediately
                                                             // the receiver was consumed and dropped by pump → upstream send fails
        assert!(tx.send(done_event()).await.is_err());
    }
}
