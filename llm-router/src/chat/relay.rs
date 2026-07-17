//! Relay seams over iii streaming channels. The router never hands a provider
//! the caller's channel: per attempt it mints a fresh iii channel (via
//! `ChannelFactory`), reads provider frames itself (`RelayRead`), and forwards
//! each frame to the caller's writer (`FrameSink`).
use std::sync::Arc;
use std::time::Duration;

use iii_sdk::channel::StreamChannelRef;

#[derive(Debug)]
pub enum ReadEvent {
    Msg(String),
    Eof,
    Timeout,
}

/// What the relay loop reads. Implemented by the fake channel (testkit) and
/// the real SDK adapter (channels.rs, Task 18).
#[async_trait::async_trait]
pub trait RelayRead: Send {
    async fn next(&mut self, timeout: Duration) -> ReadEvent;
    /// Shared close handle — router::abort closes the provider channel from
    /// another task while the relay loop is blocked in next().
    fn closer(&self) -> Arc<dyn Fn() + Send + Sync>;
    fn close(&self);
}

#[derive(Debug, thiserror::Error)]
#[error("caller channel closed")]
pub struct CallerGone;

/// What the relay loop writes (the caller's channel, or an internal one for
/// router::complete). send() fails once the peer reader dropped.
pub trait FrameSink: Send + Sync {
    fn send(&self, msg: &str) -> Result<(), CallerGone>;
    fn close(&self);
    fn is_closed(&self) -> bool;
}

/// A router-owned channel, fresh per provider attempt (relay topology).
/// Minted by `channels::create_router_channel` (real iii channel) or
/// `testkit::fake_channels` (relay-loop tests).
pub struct RouterChannel {
    pub writer_ref: StreamChannelRef,
    pub reader: Box<dyn RelayRead>,
    pub writer: Arc<dyn FrameSink>, // used by router::complete's internal channel
}

use crate::types::events::{AssistantMessageEvent, Usage};
use crate::types::messages::AssistantMessage;
use crate::types::model::Pricing;
use std::sync::atomic::{AtomicBool, Ordering};

use super::pricing::fill_cost_usd;

#[derive(Debug)]
pub enum NoTerminalReason {
    Closed,
    Idle,
}

#[derive(Debug)]
pub enum RelayResult {
    Done {
        terminal: AssistantMessageEvent,
        forwarded: bool,
    },
    ErrorFrame {
        terminal: AssistantMessageEvent,
        forwarded: bool,
        terminal_forwarded: bool,
    },
    NoTerminal {
        reason: NoTerminalReason,
        partial: Option<AssistantMessage>,
        usage: Option<Usage>,
        forwarded: bool,
    },
    Aborted {
        partial: Option<AssistantMessage>,
        usage: Option<Usage>,
        forwarded: bool,
    },
    CallerGone {
        forwarded: bool,
    },
}

pub struct RelayOpts {
    pub idle: std::time::Duration,
    pub pricing: Option<Pricing>,
    pub aborted: Arc<AtomicBool>,
}

/// One provider attempt (design § chat flow step 5). Reads provider frames,
/// enforces the idle budget, fills cost_usd, forwards in order, tracks the
/// cumulative message (boundary snapshots + accumulated deltas), classifies
/// how the stream ended.
///
/// Idle semantics: before any content, every frame (incl. ping) resets the
/// budget — a slow first token behind provider keepalives is legitimate
/// (local llama.cpp prompt eval). Once content has started, only non-ping
/// frames reset it: a provider pinging past a dead upstream must trip the
/// idle guard, not zombie on until the engine's stream_timeout kills the
/// call ("stream ended without a terminal frame").
pub async fn relay_frames(
    reader: &mut Box<dyn RelayRead>,
    sink: &dyn FrameSink,
    opts: &RelayOpts,
) -> RelayResult {
    let mut forwarded = false; // a non-ping frame reached the caller (gates retry)
                               // Cumulative message for abort/no-terminal synthesis: slim deltas are
                               // cheap string appends; legacy fat partials replace the snapshot.
    let mut acc = crate::chat::accumulate::PartialAccumulator::default();
    let mut usage: Option<Usage> = None;
    let mut content_started = false;
    let mut last_progress = std::time::Instant::now();

    loop {
        let budget = if content_started {
            opts.idle.saturating_sub(last_progress.elapsed())
        } else {
            opts.idle
        };
        let read = if budget.is_zero() {
            ReadEvent::Timeout
        } else {
            reader.next(budget).await
        };

        if opts.aborted.load(Ordering::SeqCst) {
            reader.close();
            return RelayResult::Aborted {
                partial: acc.current(),
                usage,
                forwarded,
            };
        }
        match read {
            ReadEvent::Eof => {
                return RelayResult::NoTerminal {
                    reason: NoTerminalReason::Closed,
                    partial: acc.current(),
                    usage,
                    forwarded,
                }
            }
            ReadEvent::Timeout => {
                reader.close();
                return RelayResult::NoTerminal {
                    reason: NoTerminalReason::Idle,
                    partial: acc.current(),
                    usage,
                    forwarded,
                };
            }
            ReadEvent::Msg(msg) => {
                let Ok(ev) = serde_json::from_str::<AssistantMessageEvent>(&msg) else {
                    continue; // malformed frame: skip, never fatal
                };
                if !matches!(ev, AssistantMessageEvent::Ping) {
                    last_progress = std::time::Instant::now();
                    // Start carries an empty partial pre-content; it must not
                    // arm the strict budget or slow-first-token providers trip.
                    if ev.is_content() && !matches!(ev, AssistantMessageEvent::Start { .. }) {
                        content_started = true;
                    }
                }
                acc.apply(&ev);
                let is_ping = matches!(ev, AssistantMessageEvent::Ping);

                // Only Usage/Done/Error are enriched (cost_usd) and need
                // re-serialization; every other frame is forwarded as the
                // original string — no per-frame re-serialize on the hot path.
                let enriched = match ev {
                    AssistantMessageEvent::Usage { usage: u } => {
                        let filled = fill_cost_usd(&u, opts.pricing.as_ref());
                        usage = Some(filled.clone());
                        Some(AssistantMessageEvent::Usage { usage: filled })
                    }
                    AssistantMessageEvent::Done { mut message } => {
                        message.usage = message
                            .usage
                            .map(|u| fill_cost_usd(&u, opts.pricing.as_ref()));
                        Some(AssistantMessageEvent::Done { message })
                    }
                    AssistantMessageEvent::Error { mut error } => {
                        error.usage = error
                            .usage
                            .map(|u| fill_cost_usd(&u, opts.pricing.as_ref()))
                            .or_else(|| usage.clone());
                        Some(AssistantMessageEvent::Error { error })
                    }
                    _ => None,
                };

                let is_done = matches!(enriched, Some(AssistantMessageEvent::Done { .. }));
                let is_error = matches!(enriched, Some(AssistantMessageEvent::Error { .. }));
                // Terminal-holdback: a pre-content error terminal stays with us
                // so chat.rs can retry the attempt invisibly.
                if is_error && !forwarded {
                    reader.close();
                    return RelayResult::ErrorFrame {
                        terminal: enriched.expect("error frame just matched"),
                        forwarded: false,
                        terminal_forwarded: false,
                    };
                }

                let frame = match &enriched {
                    Some(out) => std::borrow::Cow::Owned(
                        serde_json::to_string(out).expect("serializable frame"),
                    ),
                    None => std::borrow::Cow::Borrowed(msg.as_str()),
                };
                if sink.send(&frame).is_err() {
                    reader.close(); // closure propagates: the provider's writes now fail
                    return RelayResult::CallerGone { forwarded };
                }
                if !is_ping {
                    forwarded = true;
                }
                if is_done {
                    return RelayResult::Done {
                        terminal: enriched.expect("done frame just matched"),
                        forwarded,
                    };
                }
                if is_error {
                    return RelayResult::ErrorFrame {
                        terminal: enriched.expect("error frame just matched"),
                        forwarded,
                        terminal_forwarded: true,
                    };
                }
            }
        }
    }
}

#[cfg(test)]
mod loop_tests {
    use super::*;
    use crate::chat::pricing;
    use crate::testkit::fake_channels::FakeChannel;
    use crate::types::events::{AssistantMessageEvent, ErrorKind, StopReason, Usage};
    use crate::types::messages::AssistantMessage;
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;
    use std::time::Duration;

    fn partial(text: &str) -> AssistantMessage {
        let mut m = crate::chat::synthesize::empty_partial("m", "p", 1);
        if !text.is_empty() {
            m.content
                .push(crate::types::content::ContentBlock::Text { text: text.into() });
        }
        m
    }
    fn send(ch: &FakeChannel, ev: &AssistantMessageEvent) {
        ch.writer.send(&serde_json::to_string(ev).unwrap()).unwrap();
    }
    async fn drain(caller: &mut FakeChannel) -> Vec<AssistantMessageEvent> {
        let mut out = vec![];
        while let ReadEvent::Msg(m) = caller.reader.next(Duration::from_millis(50)).await {
            out.push(serde_json::from_str(&m).unwrap());
        }
        out
    }

    fn opts(idle_ms: u64) -> (Arc<AtomicBool>, RelayOpts) {
        let aborted = Arc::new(AtomicBool::new(false));
        (
            aborted.clone(),
            RelayOpts {
                idle: Duration::from_millis(idle_ms),
                pricing: None,
                aborted,
            },
        )
    }

    #[tokio::test]
    async fn forwards_in_order_fills_cost_and_returns_done_terminal() {
        let provider = FakeChannel::new();
        let mut caller = FakeChannel::new();
        send(
            &provider,
            &AssistantMessageEvent::Start {
                partial: partial(""),
            },
        );
        send(
            &provider,
            &AssistantMessageEvent::Usage {
                usage: Usage {
                    input: Some(3),
                    ..Default::default()
                },
            },
        );
        let mut done_msg = partial("hi");
        done_msg.usage = Some(Usage {
            input: Some(3),
            output: Some(2),
            ..Default::default()
        });
        send(
            &provider,
            &AssistantMessageEvent::Done { message: done_msg },
        );
        provider.writer.close();

        let (_, mut o) = opts(1000);
        o.pricing = Some(crate::types::model::Pricing {
            input: Some(3.0),
            output: Some(15.0),
            ..Default::default()
        });
        let mut reader: Box<dyn RelayRead> = Box::new(provider.reader);
        let result = relay_frames(&mut reader, &caller.writer.clone(), &o).await;

        let RelayResult::Done {
            terminal,
            forwarded,
        } = result
        else {
            panic!("want done, got {result:?}")
        };
        assert!(forwarded);
        let AssistantMessageEvent::Done { message } = &terminal else {
            panic!()
        };
        let cost = message.usage.as_ref().unwrap().cost_usd.unwrap();
        assert!(
            (cost
                - pricing::fill_cost_usd(message.usage.as_ref().unwrap(), None)
                    .cost_usd
                    .unwrap_or(cost))
            .abs()
                < 1e-12
        );
        let frames = drain(&mut caller).await;
        assert_eq!(frames.len(), 3);
        assert!(matches!(frames[2], AssistantMessageEvent::Done { .. }));
    }

    #[tokio::test]
    async fn error_before_any_content_is_held_back_for_retry() {
        let provider = FakeChannel::new();
        let mut caller = FakeChannel::new();
        let mut err = partial("");
        err.stop_reason = StopReason::Error;
        err.error_kind = Some(ErrorKind::RateLimited);
        send(&provider, &AssistantMessageEvent::Error { error: err });

        let (_, o) = opts(1000);
        let mut reader: Box<dyn RelayRead> = Box::new(provider.reader);
        let result = relay_frames(&mut reader, &caller.writer.clone(), &o).await;
        let RelayResult::ErrorFrame {
            forwarded,
            terminal_forwarded,
            terminal,
        } = result
        else {
            panic!("got {result:?}")
        };
        assert!(!forwarded);
        assert!(!terminal_forwarded);
        let AssistantMessageEvent::Error { error } = terminal else {
            panic!()
        };
        assert_eq!(error.error_kind, Some(ErrorKind::RateLimited));
        assert!(drain(&mut caller).await.is_empty()); // the caller saw nothing
    }

    #[tokio::test]
    async fn error_after_content_is_forwarded_and_marked() {
        let provider = FakeChannel::new();
        let mut caller = FakeChannel::new();
        send(
            &provider,
            &AssistantMessageEvent::Start {
                partial: partial(""),
            },
        );
        let mut err = partial("He");
        err.stop_reason = StopReason::Error;
        err.error_kind = Some(ErrorKind::Transient);
        send(&provider, &AssistantMessageEvent::Error { error: err });

        let (_, o) = opts(1000);
        let mut reader: Box<dyn RelayRead> = Box::new(provider.reader);
        let RelayResult::ErrorFrame {
            forwarded,
            terminal_forwarded,
            ..
        } = relay_frames(&mut reader, &caller.writer.clone(), &o).await
        else {
            panic!()
        };
        assert!(forwarded && terminal_forwarded);
        assert_eq!(drain(&mut caller).await.len(), 2);
    }

    #[tokio::test]
    async fn eof_without_terminal_idle_timeout_and_ping_semantics() {
        // crash: EOF without terminal carries partial + last usage
        let provider = FakeChannel::new();
        let caller = FakeChannel::new();
        send(
            &provider,
            &AssistantMessageEvent::Start {
                partial: partial("He"),
            },
        );
        send(
            &provider,
            &AssistantMessageEvent::Usage {
                usage: Usage {
                    input: Some(9),
                    ..Default::default()
                },
            },
        );
        provider.writer.close();
        let (_, o) = opts(1000);
        let mut reader: Box<dyn RelayRead> = Box::new(provider.reader);
        let RelayResult::NoTerminal {
            reason,
            partial: p,
            usage,
            ..
        } = relay_frames(&mut reader, &caller.writer.clone(), &o).await
        else {
            panic!()
        };
        assert!(matches!(reason, NoTerminalReason::Closed));
        assert_eq!(p.unwrap().content.len(), 1);
        assert_eq!(usage.unwrap().input, Some(9));

        // idle: pre-content pings reset the clock and are forwarded but never
        // count as content (slow first token behind keepalives is legitimate)
        let provider = FakeChannel::new();
        let caller = FakeChannel::new();
        let (_, o) = opts(80);
        let writer = provider.writer.clone();
        tokio::spawn(async move {
            for _ in 0..3 {
                tokio::time::sleep(Duration::from_millis(50)).await;
                let _ = writer.send(&serde_json::to_string(&AssistantMessageEvent::Ping).unwrap());
            }
            // then silence > 80ms → idle trip
        });
        let mut reader: Box<dyn RelayRead> = Box::new(provider.reader);
        let RelayResult::NoTerminal {
            reason, forwarded, ..
        } = relay_frames(&mut reader, &caller.writer.clone(), &o).await
        else {
            panic!()
        };
        assert!(matches!(reason, NoTerminalReason::Idle));
        assert!(!forwarded);
    }

    #[tokio::test]
    async fn pings_do_not_extend_idle_once_content_started() {
        // A stalled upstream behind a pinging provider must trip the idle
        // guard — not zombie on until the engine's stream_timeout kills the
        // call as "stream ended without a terminal frame".
        let provider = FakeChannel::new();
        let caller = FakeChannel::new();
        send(
            &provider,
            &AssistantMessageEvent::Start {
                partial: partial(""),
            },
        );
        send(
            &provider,
            &AssistantMessageEvent::TextDelta {
                partial: Some(partial("hi")),
                delta: "hi".into(),
            },
        );
        let writer = provider.writer.clone();
        let feeder = tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_millis(20)).await;
                if writer
                    .send(&serde_json::to_string(&AssistantMessageEvent::Ping).unwrap())
                    .is_err()
                {
                    break;
                }
            }
        });
        let (_, o) = opts(100);
        let mut reader: Box<dyn RelayRead> = Box::new(provider.reader);
        let result = relay_frames(&mut reader, &caller.writer.clone(), &o).await;
        feeder.abort();
        let RelayResult::NoTerminal {
            reason, forwarded, ..
        } = result
        else {
            panic!("want no-terminal, got {result:?}")
        };
        assert!(matches!(reason, NoTerminalReason::Idle));
        assert!(forwarded);
    }

    #[tokio::test]
    async fn caller_gone_closes_the_provider_reader_and_abort_is_observed() {
        let provider = FakeChannel::new();
        let caller = FakeChannel::new();
        caller.reader.close(); // consumer abandoned the turn
        send(
            &provider,
            &AssistantMessageEvent::Start {
                partial: partial(""),
            },
        );
        let (_, o) = opts(1000);
        let mut reader: Box<dyn RelayRead> = Box::new(provider.reader);
        let result = relay_frames(&mut reader, &caller.writer.clone(), &o).await;
        assert!(matches!(result, RelayResult::CallerGone { .. }));
        assert!(provider.writer.send("x").is_err()); // closure propagated

        // abort flag → Aborted with the partial
        let provider = FakeChannel::new();
        let caller = FakeChannel::new();
        send(
            &provider,
            &AssistantMessageEvent::Start {
                partial: partial("Hel"),
            },
        );
        let (aborted, o) = opts(1000);
        let mut reader: Box<dyn RelayRead> = Box::new(provider.reader);
        let closer = reader.closer();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(20)).await;
            aborted.store(true, std::sync::atomic::Ordering::SeqCst);
            closer();
        });
        let RelayResult::Aborted { partial: p, .. } =
            relay_frames(&mut reader, &caller.writer.clone(), &o).await
        else {
            panic!()
        };
        assert_eq!(p.unwrap().content.len(), 1);
    }

    #[tokio::test]
    async fn slim_delta_stream_synthesis_carries_the_accumulated_text() {
        // A provider dies mid-block after slim (partial-less) deltas: the
        // no-terminal result must still carry every delta received.
        let provider = FakeChannel::new();
        let caller = FakeChannel::new();
        send(
            &provider,
            &AssistantMessageEvent::Start {
                partial: partial(""),
            },
        );
        send(
            &provider,
            &AssistantMessageEvent::TextStart {
                partial: partial(""),
            },
        );
        send(
            &provider,
            &AssistantMessageEvent::TextDelta {
                partial: None,
                delta: "Hel".into(),
            },
        );
        send(
            &provider,
            &AssistantMessageEvent::TextDelta {
                partial: None,
                delta: "lo".into(),
            },
        );
        provider.writer.close(); // crash: no terminal frame
        let (_, o) = opts(1000);
        let mut reader: Box<dyn RelayRead> = Box::new(provider.reader);
        let RelayResult::NoTerminal { partial: p, .. } =
            relay_frames(&mut reader, &caller.writer.clone(), &o).await
        else {
            panic!("want no-terminal");
        };
        let msg = p.expect("accumulated partial");
        assert_eq!(
            msg.content,
            vec![crate::types::content::ContentBlock::Text {
                text: "Hello".into()
            }]
        );
    }

    #[tokio::test]
    async fn non_enriched_frames_are_forwarded_byte_identical() {
        // The relay must not re-serialize pass-through frames: the caller
        // receives the provider's exact bytes (key order, whitespace, all).
        let provider = FakeChannel::new();
        let mut caller = FakeChannel::new();
        let odd_spacing = r#"{ "type": "text_delta",   "delta": "hi" }"#;
        provider.writer.send(odd_spacing).unwrap();
        let done = serde_json::to_string(&AssistantMessageEvent::Done {
            message: partial("hi"),
        })
        .unwrap();
        provider.writer.send(&done).unwrap();
        provider.writer.close();

        let (_, o) = opts(1000);
        let mut reader: Box<dyn RelayRead> = Box::new(provider.reader);
        let result = relay_frames(&mut reader, &caller.writer.clone(), &o).await;
        assert!(matches!(result, RelayResult::Done { .. }));

        let mut raw = vec![];
        while let ReadEvent::Msg(m) = caller.reader.next(Duration::from_millis(50)).await {
            raw.push(m);
        }
        assert_eq!(raw[0], odd_spacing, "pass-through frame was rewritten");
    }
}
