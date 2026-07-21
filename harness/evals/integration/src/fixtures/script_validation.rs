use anyhow::Context;

use crate::types::script::{JsonMatcherV1, JsonNormalizerV1, NormalizerOperation, RouterScriptV1};

pub fn validate_script(script: &RouterScriptV1) -> anyhow::Result<()> {
    if script.generations.is_empty() {
        anyhow::bail!("script has no generations");
    }
    let mut seen = std::collections::BTreeSet::new();
    for generation in &script.generations {
        if !seen.insert(generation.ordinal) {
            anyhow::bail!("duplicate generation ordinal {}", generation.ordinal);
        }
        for (field, matcher) in generation.match_.fields() {
            validate_matcher(matcher)
                .with_context(|| format!("generation {} field {field}", generation.ordinal))?;
        }
        let terminal_positions: Vec<usize> = generation
            .frames
            .iter()
            .enumerate()
            .filter(|(_, frame)| frame.is_terminal())
            .map(|(index, _)| index)
            .collect();
        match terminal_positions.as_slice() {
            [] => anyhow::bail!("generation {} has no terminal frame", generation.ordinal),
            [last] if *last == generation.frames.len() - 1 => {}
            [_] => anyhow::bail!(
                "generation {}: terminal frame is not the last frame",
                generation.ordinal
            ),
            _ => anyhow::bail!(
                "generation {} has multiple terminal frames",
                generation.ordinal
            ),
        }
        validate_response_agreement(generation)?;
    }
    Ok(())
}

fn validate_matcher(matcher: &JsonMatcherV1) -> anyhow::Result<()> {
    match matcher {
        JsonMatcherV1::Regex { pattern } => {
            regex::Regex::new(pattern)
                .map_err(|error| anyhow::anyhow!("invalid regex {pattern:?}: {error}"))?;
        }
        JsonMatcherV1::Sha256 { expected } => {
            let is_placeholder = expected.contains("{{");
            let is_hex =
                expected.len() == 64 && expected.bytes().all(|byte| byte.is_ascii_hexdigit());
            if !is_placeholder && !is_hex {
                anyhow::bail!("sha256 expected value must be 64 hex chars, got {expected:?}");
            }
        }
        JsonMatcherV1::Exact { normalize, .. } | JsonMatcherV1::Subset { normalize, .. } => {
            for normalizer in normalize.as_deref().unwrap_or_default() {
                validate_normalizer(normalizer)?;
            }
        }
        JsonMatcherV1::Absent | JsonMatcherV1::Present => {}
    }
    Ok(())
}

fn validate_normalizer(normalizer: &JsonNormalizerV1) -> anyhow::Result<()> {
    crate::matcher::validate_pointer(&normalizer.pointer)?;
    match normalizer.operation {
        NormalizerOperation::Replace if normalizer.replacement.is_none() => {
            anyhow::bail!(
                "replace normalizer at {:?} requires `replacement`",
                normalizer.pointer
            )
        }
        NormalizerOperation::Delete if normalizer.replacement.is_some() => {
            anyhow::bail!(
                "delete normalizer at {:?} forbids `replacement`",
                normalizer.pointer
            )
        }
        NormalizerOperation::Delete if normalizer.pointer.is_empty() => {
            anyhow::bail!("delete normalizer cannot target the document root")
        }
        _ => Ok(()),
    }
}

/// Terminal frame and scripted response must agree (spec: "response/frame
/// disagreement" is rejected at load): `done` requires `ok:true` and a
/// matching `stop_reason`; `error` requires `ok:false` with an error shape.
fn validate_response_agreement(
    generation: &crate::types::script::ScriptedGenerationV1,
) -> anyhow::Result<()> {
    use crate::types::frames::AssistantMessageEvent as Frame;

    let terminal = generation.frames.last().expect("validated non-empty");
    match terminal {
        Frame::Done { message } => {
            if !generation.response.ok {
                anyhow::bail!(
                    "generation {}: done frame with ok:false response",
                    generation.ordinal
                );
            }
            if generation.response.error.is_some() {
                anyhow::bail!(
                    "generation {}: successful response must not carry an error",
                    generation.ordinal
                );
            }
            validate_terminal_message(generation, message)?;
            validate_stream_agreement(generation, message)?;
        }
        Frame::Error { error } => {
            if generation.response.ok || generation.response.error.is_none() {
                anyhow::bail!(
                    "generation {}: error frame requires ok:false and an error shape",
                    generation.ordinal
                );
            }
            validate_terminal_message(generation, error)?;
            let response_error = generation.response.error.as_ref().expect("checked");
            let message_error = error.error_message.as_deref();
            if message_error != Some(response_error.message.as_str()) {
                anyhow::bail!(
                    "generation {}: response error message disagrees with terminal error frame",
                    generation.ordinal
                );
            }
            let message_code = error.error_kind.map(|kind| {
                serde_json::to_value(kind)
                    .expect("error kind serializes")
                    .as_str()
                    .expect("error kind is a string")
                    .to_string()
            });
            if message_code.as_deref() != Some(response_error.code.as_str()) {
                anyhow::bail!(
                    "generation {}: response error code disagrees with terminal error frame",
                    generation.ordinal
                );
            }
            validate_stream_agreement(generation, error)?;
        }
        _ => unreachable!("terminal position validated"),
    }
    Ok(())
}

fn validate_terminal_message(
    generation: &crate::types::script::ScriptedGenerationV1,
    message: &crate::types::frames::AssistantMessage,
) -> anyhow::Result<()> {
    if generation.response.provider != message.provider {
        anyhow::bail!(
            "generation {}: response provider disagrees with terminal message",
            generation.ordinal
        );
    }
    if generation.response.model != message.model {
        anyhow::bail!(
            "generation {}: response model disagrees with terminal message",
            generation.ordinal
        );
    }
    if generation.response.stop_reason != Some(message.stop_reason) {
        anyhow::bail!(
            "generation {}: response stop_reason disagrees with terminal message",
            generation.ordinal
        );
    }
    if generation.response.usage != message.usage {
        anyhow::bail!(
            "generation {}: response usage disagrees with terminal message",
            generation.ordinal
        );
    }
    Ok(())
}

fn validate_stream_agreement(
    generation: &crate::types::script::ScriptedGenerationV1,
    terminal: &crate::types::frames::AssistantMessage,
) -> anyhow::Result<()> {
    use crate::types::frames::AssistantMessageEvent as Frame;

    let mut streamed_usage = None;
    let mut streamed_stop = None;

    for frame in &generation.frames[..generation.frames.len() - 1] {
        let partial = match frame {
            Frame::Start { partial }
            | Frame::TextStart { partial }
            | Frame::TextEnd { partial }
            | Frame::ThinkingStart { partial }
            | Frame::ThinkingEnd { partial }
            | Frame::FunctioncallStart { partial }
            | Frame::FunctioncallEnd { partial } => Some(partial),
            Frame::TextDelta {
                partial: Some(partial),
                ..
            }
            | Frame::ThinkingDelta {
                partial: Some(partial),
                ..
            }
            | Frame::FunctioncallDelta {
                partial: Some(partial),
                ..
            } => Some(partial),
            _ => None,
        };
        if let Some(partial) = partial {
            if partial.provider != terminal.provider || partial.model != terminal.model {
                anyhow::bail!(
                    "generation {}: streamed partial provider/model disagrees with terminal message",
                    generation.ordinal
                );
            }
        }

        match frame {
            Frame::Usage { usage } => streamed_usage = Some(usage.clone()),
            Frame::Stop {
                stop_reason,
                error_message,
                error_kind,
            } => {
                streamed_stop = Some((*stop_reason, error_message.clone(), *error_kind));
            }
            _ => {}
        }
    }

    super::stream_validation::validate_stream_content(
        &generation.frames[..generation.frames.len() - 1],
        terminal,
        generation.ordinal,
    )?;
    if let Some(usage) = streamed_usage {
        if terminal.usage.as_ref() != Some(&usage) {
            anyhow::bail!(
                "generation {}: streamed usage disagrees with terminal message",
                generation.ordinal
            );
        }
    }
    if let Some((stop_reason, error_message, error_kind)) = streamed_stop {
        if stop_reason != terminal.stop_reason
            || error_message != terminal.error_message
            || error_kind != terminal.error_kind
        {
            anyhow::bail!(
                "generation {}: stop frame disagrees with terminal message",
                generation.ordinal
            );
        }
    }
    Ok(())
}
