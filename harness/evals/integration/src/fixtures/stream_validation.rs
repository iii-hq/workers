//! Content-level agreement between streamed frames and the terminal message.
//!
//! Boundary snapshots are authoritative for reconstruction, but they must not
//! hide contradictory deltas that consumers already observed. Validate each
//! open block before accepting its end snapshot, then compare the fully
//! reconstructed content with the terminal frame.

use crate::types::frames::{AssistantMessage, AssistantMessageEvent as Frame, ContentBlock};

#[derive(Default)]
struct StreamState {
    base: Option<AssistantMessage>,
    open: Option<OpenBlock>,
    saw_content: bool,
}

enum OpenBlock {
    Text { seed: String, deltas: String },
    Thinking { seed: String, deltas: String },
    FunctionCall { id: String, arguments: String },
}

pub(super) fn validate_stream_content(
    frames: &[Frame],
    terminal: &AssistantMessage,
    ordinal: u64,
) -> anyhow::Result<()> {
    let mut state = StreamState::default();
    for frame in frames {
        state.apply(frame, ordinal)?;
    }
    if !state.saw_content {
        return Ok(());
    }

    let reconstructed = state.reconstructed_content(ordinal)?;
    if !content_equal_allowing_pending_thinking_signature(&reconstructed, &terminal.content) {
        anyhow::bail!("generation {ordinal}: streamed content disagrees with terminal message");
    }
    Ok(())
}

impl StreamState {
    fn apply(&mut self, frame: &Frame, ordinal: u64) -> anyhow::Result<()> {
        match frame {
            Frame::Start { partial } => {
                self.base = Some(partial.clone());
                self.open = None;
                self.saw_content |= !partial.content.is_empty();
            }
            Frame::TextStart { partial } => {
                self.start_text(partial);
            }
            Frame::TextDelta {
                partial: Some(partial),
                ..
            } => {
                self.base = Some(partial.clone());
                self.open = None;
                self.saw_content = true;
            }
            Frame::TextDelta {
                partial: None,
                delta,
            } => {
                self.saw_content = true;
                match &mut self.open {
                    Some(OpenBlock::Text { deltas, .. }) => deltas.push_str(delta),
                    Some(_) => anyhow::bail!(
                        "generation {ordinal}: text delta arrived while another block was open"
                    ),
                    None => {
                        self.open = Some(OpenBlock::Text {
                            seed: String::new(),
                            deltas: delta.clone(),
                        });
                    }
                }
            }
            Frame::TextEnd { partial } => {
                self.validate_boundary(partial, BoundaryKind::Text, ordinal)?;
                self.base = Some(partial.clone());
                self.open = None;
                self.saw_content = true;
            }
            Frame::ThinkingStart { partial } => {
                self.start_thinking(partial);
            }
            Frame::ThinkingDelta {
                partial: Some(partial),
                ..
            } => {
                self.base = Some(partial.clone());
                self.open = None;
                self.saw_content = true;
            }
            Frame::ThinkingDelta {
                partial: None,
                delta,
            } => {
                self.saw_content = true;
                match &mut self.open {
                    Some(OpenBlock::Thinking { deltas, .. }) => deltas.push_str(delta),
                    Some(_) => anyhow::bail!(
                        "generation {ordinal}: thinking delta arrived while another block was open"
                    ),
                    None => {
                        self.open = Some(OpenBlock::Thinking {
                            seed: String::new(),
                            deltas: delta.clone(),
                        });
                    }
                }
            }
            Frame::ThinkingEnd { partial } => {
                self.validate_boundary(partial, BoundaryKind::Thinking, ordinal)?;
                self.base = Some(partial.clone());
                self.open = None;
                self.saw_content = true;
            }
            Frame::FunctioncallStart { partial } => {
                self.base = Some(partial.clone());
                self.open = Some(OpenBlock::FunctionCall {
                    id: String::new(),
                    arguments: String::new(),
                });
                self.saw_content = true;
            }
            Frame::FunctioncallDelta {
                partial: Some(partial),
                ..
            } => {
                self.base = Some(partial.clone());
                self.open = None;
                self.saw_content = true;
            }
            Frame::FunctioncallDelta {
                partial: None,
                delta,
                id,
            } => {
                self.saw_content = true;
                match &mut self.open {
                    Some(OpenBlock::FunctionCall {
                        id: open_id,
                        arguments,
                    }) => {
                        if !id.is_empty() {
                            if !open_id.is_empty() && open_id != id {
                                anyhow::bail!(
                                    "generation {ordinal}: function-call delta changed call id"
                                );
                            }
                            *open_id = id.clone();
                        }
                        arguments.push_str(delta);
                    }
                    Some(_) => anyhow::bail!(
                        "generation {ordinal}: function-call delta arrived while another block was open"
                    ),
                    None => {
                        self.open = Some(OpenBlock::FunctionCall {
                            id: id.clone(),
                            arguments: delta.clone(),
                        });
                    }
                }
            }
            Frame::FunctioncallEnd { partial } => {
                self.validate_boundary(partial, BoundaryKind::FunctionCall, ordinal)?;
                self.base = Some(partial.clone());
                self.open = None;
                self.saw_content = true;
            }
            Frame::Usage { .. }
            | Frame::Ping
            | Frame::Stop { .. }
            | Frame::Done { .. }
            | Frame::Error { .. } => {}
        }
        Ok(())
    }

    fn start_text(&mut self, partial: &AssistantMessage) {
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
            deltas: String::new(),
        });
        self.saw_content = true;
    }

    fn start_thinking(&mut self, partial: &AssistantMessage) {
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
            deltas: String::new(),
        });
        self.saw_content = true;
    }

    fn validate_boundary(
        &self,
        partial: &AssistantMessage,
        expected_kind: BoundaryKind,
        ordinal: u64,
    ) -> anyhow::Result<()> {
        let Some(open) = &self.open else {
            return Ok(());
        };
        if open.kind() != expected_kind {
            anyhow::bail!("generation {ordinal}: stream block ended with the wrong frame type");
        }
        let reconstructed = self.reconstructed_content(ordinal)?;
        let agrees = match expected_kind {
            BoundaryKind::Thinking => {
                content_equal_ignoring_thinking_signatures(&reconstructed, &partial.content)
            }
            BoundaryKind::Text | BoundaryKind::FunctionCall => reconstructed == partial.content,
        };
        if !agrees {
            anyhow::bail!(
                "generation {ordinal}: streamed deltas disagree with the block-end snapshot"
            );
        }
        Ok(())
    }

    fn reconstructed_content(&self, ordinal: u64) -> anyhow::Result<Vec<ContentBlock>> {
        let mut content = self
            .base
            .as_ref()
            .map(|base| base.content.clone())
            .unwrap_or_default();
        match &self.open {
            None => {}
            Some(OpenBlock::Text { seed, deltas }) => content.push(ContentBlock::Text {
                text: format!("{seed}{deltas}"),
            }),
            Some(OpenBlock::Thinking { seed, deltas }) => {
                content.push(ContentBlock::Thinking {
                    text: format!("{seed}{deltas}"),
                    signature: None,
                });
            }
            Some(OpenBlock::FunctionCall { id, arguments }) => {
                let parsed = serde_json::from_str::<serde_json::Value>(arguments)
                    .map_err(|error| {
                        anyhow::anyhow!(
                            "generation {ordinal}: streamed function-call arguments are not complete JSON: {error}"
                        )
                    })?;
                if !parsed.is_object() {
                    anyhow::bail!(
                        "generation {ordinal}: streamed function-call arguments must be an object"
                    );
                }
                let Some(ContentBlock::FunctionCall {
                    id: base_id,
                    arguments: base_arguments,
                    ..
                }) = content
                    .iter_mut()
                    .rev()
                    .find(|block| matches!(block, ContentBlock::FunctionCall { .. }))
                else {
                    anyhow::bail!(
                        "generation {ordinal}: function-call deltas require a start snapshot"
                    );
                };
                if !id.is_empty() && base_id != id {
                    anyhow::bail!(
                        "generation {ordinal}: streamed function-call id disagrees with its snapshot"
                    );
                }
                *base_arguments = parsed;
            }
        }
        Ok(content)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BoundaryKind {
    Text,
    Thinking,
    FunctionCall,
}

impl OpenBlock {
    fn kind(&self) -> BoundaryKind {
        match self {
            OpenBlock::Text { .. } => BoundaryKind::Text,
            OpenBlock::Thinking { .. } => BoundaryKind::Thinking,
            OpenBlock::FunctionCall { .. } => BoundaryKind::FunctionCall,
        }
    }
}

fn content_equal_allowing_pending_thinking_signature(
    reconstructed: &[ContentBlock],
    terminal: &[ContentBlock],
) -> bool {
    content_equal_ignoring_thinking_signatures(reconstructed, terminal)
}

fn content_equal_ignoring_thinking_signatures(
    left: &[ContentBlock],
    right: &[ContentBlock],
) -> bool {
    let strip = |blocks: &[ContentBlock]| {
        blocks
            .iter()
            .cloned()
            .map(|block| match block {
                ContentBlock::Thinking { text, .. } => ContentBlock::Thinking {
                    text,
                    signature: None,
                },
                block => block,
            })
            .collect::<Vec<_>>()
    };
    strip(left) == strip(right)
}
