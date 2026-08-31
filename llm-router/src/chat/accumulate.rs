//! Cumulative-message reconstruction over the streaming vocabulary.
//!
//! Delta frames carry only their `delta` (a per-chunk cumulative snapshot
//! made streams O(N²) — see `types::events`); block-boundary frames carry
//! authoritative snapshots. This accumulator folds the two back into "the
//! message so far": the last boundary snapshot plus the accumulated deltas
//! of the open block. The relay feeds abort / no-terminal synthesis from
//! it, so a provider dying mid-block still yields every delta received.
//!
//! Legacy fat deltas (`partial: Some`) replace the snapshot wholesale —
//! byte-identical to the old behavior, so old producers interop.

use crate::types::content::ContentBlock;
use crate::types::events::{AssistantMessageEvent, FunctionCallArgumentsPreview};
use crate::types::messages::{degraded_arguments, AssistantMessage};

#[derive(Default)]
pub struct PartialAccumulator {
    /// Last block-boundary snapshot (authoritative: signatures, final args).
    base: Option<AssistantMessage>,
    /// Deltas since that snapshot.
    open: Option<OpenBlock>,
}

enum OpenBlock {
    Text {
        seed: String,
        acc: String,
    },
    Thinking {
        seed: String,
        acc: String,
    },
    Call {
        raw: String,
        stream: crate::json_stream::JsonStream,
    },
}

impl PartialAccumulator {
    pub fn apply(&mut self, event: &AssistantMessageEvent) {
        match event {
            AssistantMessageEvent::Start { partial }
            | AssistantMessageEvent::TextEnd { partial }
            | AssistantMessageEvent::ThinkingEnd { partial }
            | AssistantMessageEvent::FunctioncallEnd { partial } => {
                self.base = Some(partial.clone());
                self.open = None;
            }
            // Producers differ on whether the Start snapshot already carries
            // the just-opened (empty) block — pop a trailing match into the
            // seed so the merge never duplicates it.
            AssistantMessageEvent::TextStart { partial } => {
                let mut base = partial.clone();
                let seed = match base.content.last() {
                    Some(ContentBlock::Text { text }) => {
                        let text = text.clone();
                        base.content.pop();
                        text
                    }
                    _ => String::new(),
                };
                self.base = Some(base);
                self.open = Some(OpenBlock::Text {
                    seed,
                    acc: String::new(),
                });
            }
            AssistantMessageEvent::ThinkingStart { partial } => {
                let mut base = partial.clone();
                let seed = match base.content.last() {
                    Some(ContentBlock::Thinking { text, .. }) => {
                        let text = text.clone();
                        base.content.pop();
                        text
                    }
                    _ => String::new(),
                };
                self.base = Some(base);
                self.open = Some(OpenBlock::Thinking {
                    seed,
                    acc: String::new(),
                });
            }
            // The Start snapshot keeps the call block (placeholder args);
            // deltas accumulate the raw argument text for the merge.
            AssistantMessageEvent::FunctioncallStart { partial } => {
                self.base = Some(partial.clone());
                self.open = Some(OpenBlock::Call {
                    raw: String::new(),
                    stream: crate::json_stream::JsonStream::new(),
                });
            }
            AssistantMessageEvent::TextDelta { partial, delta } => match partial {
                Some(p) => {
                    self.base = Some(p.clone());
                    self.open = None;
                }
                None => match &mut self.open {
                    Some(OpenBlock::Text { acc, .. }) => acc.push_str(delta),
                    // Defensive: producer skipped the Start frame.
                    _ => {
                        self.open = Some(OpenBlock::Text {
                            seed: String::new(),
                            acc: delta.clone(),
                        })
                    }
                },
            },
            AssistantMessageEvent::ThinkingDelta { partial, delta } => match partial {
                Some(p) => {
                    self.base = Some(p.clone());
                    self.open = None;
                }
                None => match &mut self.open {
                    Some(OpenBlock::Thinking { acc, .. }) => acc.push_str(delta),
                    _ => {
                        self.open = Some(OpenBlock::Thinking {
                            seed: String::new(),
                            acc: delta.clone(),
                        })
                    }
                },
            },
            AssistantMessageEvent::FunctioncallDelta { partial, delta, .. } => match partial {
                Some(p) => {
                    self.base = Some(p.clone());
                    self.open = None;
                }
                None => match &mut self.open {
                    Some(OpenBlock::Call { raw, stream }) => {
                        raw.push_str(delta);
                        let _ = stream.write(delta);
                    }
                    _ => {
                        let mut stream = crate::json_stream::JsonStream::new();
                        let _ = stream.write(delta);
                        self.open = Some(OpenBlock::Call {
                            raw: delta.clone(),
                            stream,
                        })
                    }
                },
            },
            AssistantMessageEvent::Usage { .. }
            | AssistantMessageEvent::Ping
            | AssistantMessageEvent::Stop { .. }
            | AssistantMessageEvent::Done { .. }
            | AssistantMessageEvent::Error { .. } => {}
        }
    }

    /// The message so far: the boundary snapshot plus the open block.
    /// `None` before any content frame.
    pub fn current(&self) -> Option<AssistantMessage> {
        let mut out = self.base.clone()?;
        match &self.open {
            Some(OpenBlock::Text { seed, acc }) if !(seed.is_empty() && acc.is_empty()) => {
                out.content.push(ContentBlock::Text {
                    text: format!("{seed}{acc}"),
                });
            }
            Some(OpenBlock::Thinking { seed, acc }) if !(seed.is_empty() && acc.is_empty()) => {
                out.content.push(ContentBlock::Thinking {
                    text: format!("{seed}{acc}"),
                    signature: None,
                });
            }
            // Mid-call view: replace the open call block's placeholder args
            // with every value observable so far. The incremental parser also
            // exposes an open string, so agent_trigger's function and
            // description no longer wait for their closing quote.
            Some(OpenBlock::Call { raw, stream }) if !raw.is_empty() => {
                if let Some(ContentBlock::FunctionCall { arguments, .. }) = out
                    .content
                    .iter_mut()
                    .rev()
                    .find(|b| matches!(b, ContentBlock::FunctionCall { .. }))
                {
                    *arguments = stream
                        .snapshot()
                        .and_then(|snapshot| match snapshot.value {
                            serde_json::Value::Object(mut map) => {
                                if !snapshot.complete {
                                    map.insert("_partial".into(), serde_json::Value::Bool(true));
                                }
                                Some(serde_json::Value::Object(map))
                            }
                            _ => None,
                        })
                        .unwrap_or_else(|| degraded_arguments(raw));
                }
            }
            _ => {}
        }
        Some(out)
    }

    /// Only the small top-level identity needed by an in-flight call card.
    /// Payload is deliberately excluded so relayed delta frames stay O(chunk)
    /// instead of growing with the cumulative arguments document.
    pub fn call_arguments_preview(&self) -> Option<FunctionCallArgumentsPreview> {
        let arguments = match &self.open {
            Some(OpenBlock::Call { stream, .. }) => stream.snapshot()?.value,
            _ => self.base.as_ref()?.content.iter().rev().find_map(|block| {
                let ContentBlock::FunctionCall { arguments, .. } = block else {
                    return None;
                };
                Some(arguments.clone())
            })?,
        };
        let object = arguments.as_object()?;
        let function = object
            .get("function")
            .and_then(serde_json::Value::as_str)
            .filter(|value| !value.is_empty())
            .map(|value| utf8_head(value, 512).to_string());
        let description = object
            .get("description")
            .and_then(serde_json::Value::as_str)
            .filter(|value| !value.is_empty())
            .map(|value| utf8_head(value, 120).to_string());
        if function.is_none() && description.is_none() {
            None
        } else {
            Some(FunctionCallArgumentsPreview {
                function,
                description,
            })
        }
    }
}

fn utf8_head(value: &str, max: usize) -> &str {
    if value.len() <= max {
        return value;
    }
    let mut end = max;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    &value[..end]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::synthesize::empty_partial;
    use crate::types::events::AssistantMessageEvent as Ev;
    use serde_json::json;

    fn base() -> AssistantMessage {
        empty_partial("m", "p", 1)
    }

    #[test]
    fn slim_text_deltas_accumulate_from_the_boundary_snapshot() {
        let mut acc = PartialAccumulator::default();
        assert!(acc.current().is_none());
        acc.apply(&Ev::Start { partial: base() });
        acc.apply(&Ev::TextStart { partial: base() });
        acc.apply(&Ev::TextDelta {
            partial: None,
            delta: "Hel".into(),
        });
        acc.apply(&Ev::TextDelta {
            partial: None,
            delta: "lo".into(),
        });
        assert_eq!(
            acc.current().unwrap().content,
            vec![ContentBlock::Text {
                text: "Hello".into()
            }]
        );
    }

    #[test]
    fn start_snapshots_with_and_without_the_open_block_merge_identically() {
        // Producer A: TextStart snapshot excludes the new (empty) block.
        let mut a = PartialAccumulator::default();
        a.apply(&Ev::TextStart { partial: base() });
        a.apply(&Ev::TextDelta {
            partial: None,
            delta: "x".into(),
        });
        // Producer B: TextStart snapshot includes the empty open block.
        let mut with_empty = base();
        with_empty.content.push(ContentBlock::Text {
            text: String::new(),
        });
        let mut b = PartialAccumulator::default();
        b.apply(&Ev::TextStart {
            partial: with_empty,
        });
        b.apply(&Ev::TextDelta {
            partial: None,
            delta: "x".into(),
        });
        assert_eq!(a.current().unwrap().content, b.current().unwrap().content);
    }

    #[test]
    fn thinking_end_snapshot_carries_the_signature() {
        let mut acc = PartialAccumulator::default();
        acc.apply(&Ev::ThinkingStart { partial: base() });
        acc.apply(&Ev::ThinkingDelta {
            partial: None,
            delta: "reasoning".into(),
        });
        // Mid-block view has no signature (it only exists at the end).
        assert_eq!(
            acc.current().unwrap().content,
            vec![ContentBlock::Thinking {
                text: "reasoning".into(),
                signature: None
            }]
        );
        let mut done = base();
        done.content = vec![ContentBlock::Thinking {
            text: "reasoning".into(),
            signature: Some("sig".into()),
        }];
        acc.apply(&Ev::ThinkingEnd {
            partial: done.clone(),
        });
        assert_eq!(acc.current().unwrap().content, done.content);
    }

    #[test]
    fn mid_call_death_yields_degraded_replay_safe_arguments() {
        let mut with_call = base();
        with_call.content = vec![ContentBlock::FunctionCall {
            id: "c1".into(),
            function_id: "agent_trigger".into(),
            arguments: json!({}),
        }];
        let mut acc = PartialAccumulator::default();
        acc.apply(&Ev::FunctioncallStart { partial: with_call });
        acc.apply(&Ev::FunctioncallDelta {
            partial: None,
            delta: r#"{"function":"state::set","payload":{"x":"#.into(),
            id: "c1".into(),
            arguments_preview: None,
        });
        assert_eq!(
            acc.call_arguments_preview(),
            Some(FunctionCallArgumentsPreview {
                function: Some("state::set".into()),
                description: None,
            })
        );
        let cum = acc.current().unwrap();
        let ContentBlock::FunctionCall { arguments, .. } = &cum.content[0] else {
            panic!("want function_call");
        };
        // Same degraded shape providers produce for incomplete args.
        assert_eq!(arguments["function"], "state::set");
        assert_eq!(arguments["_partial"], true);
    }

    #[test]
    fn complete_call_args_parse_into_a_real_object() {
        let mut with_call = base();
        with_call.content = vec![ContentBlock::FunctionCall {
            id: "c1".into(),
            function_id: "agent_trigger".into(),
            arguments: json!({}),
        }];
        let mut acc = PartialAccumulator::default();
        acc.apply(&Ev::FunctioncallStart { partial: with_call });
        acc.apply(&Ev::FunctioncallDelta {
            partial: None,
            delta: r#"{"function":"state::get"}"#.into(),
            id: "c1".into(),
            arguments_preview: None,
        });
        let cum = acc.current().unwrap();
        let ContentBlock::FunctionCall { arguments, .. } = &cum.content[0] else {
            panic!("want function_call");
        };
        assert_eq!(arguments, &json!({"function":"state::get"}));
    }

    #[test]
    fn open_call_strings_are_visible_before_their_closing_quote() {
        let mut with_call = base();
        with_call.content = vec![ContentBlock::FunctionCall {
            id: "c1".into(),
            function_id: "agent_trigger".into(),
            arguments: json!({}),
        }];
        let mut acc = PartialAccumulator::default();
        acc.apply(&Ev::FunctioncallStart { partial: with_call });
        acc.apply(&Ev::FunctioncallDelta {
            partial: None,
            delta: r#"{"function":"state::se"#.into(),
            id: "c1".into(),
            arguments_preview: None,
        });
        let cum = acc.current().unwrap();
        let ContentBlock::FunctionCall { arguments, .. } = &cum.content[0] else {
            panic!("want function_call");
        };
        assert_eq!(arguments["function"], "state::se");
        assert_eq!(arguments["_partial"], true);
    }

    #[test]
    fn legacy_fat_delta_replaces_the_snapshot_wholesale() {
        let mut fat = base();
        fat.content = vec![ContentBlock::Text {
            text: "cumulative".into(),
        }];
        let mut acc = PartialAccumulator::default();
        acc.apply(&Ev::TextDelta {
            partial: Some(fat.clone()),
            delta: "e".into(),
        });
        // The fat partial IS the message: the delta is not re-applied.
        assert_eq!(acc.current().unwrap().content, fat.content);
    }

    #[test]
    fn delta_without_a_start_frame_does_not_panic() {
        let mut acc = PartialAccumulator::default();
        acc.apply(&Ev::TextDelta {
            partial: None,
            delta: "orphan".into(),
        });
        // No boundary snapshot yet → nothing to attach the block to.
        assert!(acc.current().is_none());
        // A late Start attaches subsequent deltas normally.
        acc.apply(&Ev::Start { partial: base() });
        acc.apply(&Ev::TextDelta {
            partial: None,
            delta: "ok".into(),
        });
        assert_eq!(
            acc.current().unwrap().content,
            vec![ContentBlock::Text { text: "ok".into() }]
        );
    }
}
