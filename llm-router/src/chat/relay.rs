//! Relay seams over iii streaming channels. The router never hands a provider
//! the caller's channel: per attempt it mints a fresh iii channel (via
//! `ChannelFactory`), reads provider frames itself (`RelayRead`), and forwards
//! each frame to the caller's writer (`FrameSink`).
use std::sync::Arc;
use std::time::Duration;

use crate::types::channel::StreamChannelRef;

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

fn partial_of(ev: &AssistantMessageEvent) -> Option<&AssistantMessage> {
    use AssistantMessageEvent as E;
    match ev {
        E::Start { partial }
        | E::TextStart { partial }
        | E::TextDelta { partial, .. }
        | E::TextEnd { partial }
        | E::ThinkingStart { partial }
        | E::ThinkingDelta { partial, .. }
        | E::ThinkingEnd { partial }
        | E::FunctioncallStart { partial }
        | E::FunctioncallDelta { partial, .. }
        | E::FunctioncallEnd { partial } => Some(partial),
        _ => None,
    }
}

/// One provider attempt (design § chat flow step 5). Reads provider frames,
/// enforces the idle budget (any frame incl. ping resets it), fills cost_usd,
/// forwards in order, tracks the partial, classifies how the stream ended.
pub async fn relay_frames(
    reader: &mut Box<dyn RelayRead>,
    sink: &dyn FrameSink,
    opts: &RelayOpts,
) -> RelayResult {
    let mut forwarded = false; // a non-ping frame reached the caller (gates retry)
    let mut partial: Option<AssistantMessage> = None;
    let mut usage: Option<Usage> = None;

    loop {
        let read = reader.next(opts.idle).await;

        if opts.aborted.load(Ordering::SeqCst) {
            reader.close();
            return RelayResult::Aborted {
                partial,
                usage,
                forwarded,
            };
        }
        match read {
            ReadEvent::Eof => {
                return RelayResult::NoTerminal {
                    reason: NoTerminalReason::Closed,
                    partial,
                    usage,
                    forwarded,
                }
            }
            ReadEvent::Timeout => {
                reader.close();
                return RelayResult::NoTerminal {
                    reason: NoTerminalReason::Idle,
                    partial,
                    usage,
                    forwarded,
                };
            }
            ReadEvent::Msg(msg) => {
                let Ok(ev) = serde_json::from_str::<AssistantMessageEvent>(&msg) else {
                    continue; // malformed frame: skip, never fatal
                };
                if let Some(p) = partial_of(&ev) {
                    partial = Some(p.clone());
                }

                // Enrich usage with cost before forwarding (router fills cost_usd).
                let out = match ev {
                    AssistantMessageEvent::Usage { usage: u } => {
                        let filled = fill_cost_usd(&u, opts.pricing.as_ref());
                        usage = Some(filled.clone());
                        AssistantMessageEvent::Usage { usage: filled }
                    }
                    AssistantMessageEvent::Done { mut message } => {
                        message.usage = message
                            .usage
                            .map(|u| fill_cost_usd(&u, opts.pricing.as_ref()));
                        AssistantMessageEvent::Done { message }
                    }
                    AssistantMessageEvent::Error { mut error } => {
                        error.usage = error
                            .usage
                            .map(|u| fill_cost_usd(&u, opts.pricing.as_ref()))
                            .or_else(|| usage.clone());
                        AssistantMessageEvent::Error { error }
                    }
                    other => other,
                };

                // Terminal-holdback: a pre-content error terminal stays with us
                // so chat.rs can retry the attempt invisibly.
                let is_done = matches!(out, AssistantMessageEvent::Done { .. });
                let is_error = matches!(out, AssistantMessageEvent::Error { .. });
                if is_error && !forwarded {
                    reader.close();
                    return RelayResult::ErrorFrame {
                        terminal: out,
                        forwarded: false,
                        terminal_forwarded: false,
                    };
                }

                let is_ping = matches!(out, AssistantMessageEvent::Ping);
                if sink
                    .send(&serde_json::to_string(&out).expect("serializable frame"))
                    .is_err()
                {
                    reader.close(); // closure propagates: the provider's writes now fail
                    return RelayResult::CallerGone { forwarded };
                }
                if !is_ping {
                    forwarded = true;
                }
                if is_done {
                    return RelayResult::Done {
                        terminal: out,
                        forwarded,
                    };
                }
                if is_error {
                    return RelayResult::ErrorFrame {
                        terminal: out,
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

        // idle: pings reset the clock and are forwarded but never count as content
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
}
