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
            if let Some(stop_reason) = generation.response.stop_reason {
                if stop_reason != message.stop_reason {
                    anyhow::bail!(
                        "generation {}: response stop_reason disagrees with done message",
                        generation.ordinal
                    );
                }
            }
        }
        Frame::Error { .. } => {
            if generation.response.ok || generation.response.error.is_none() {
                anyhow::bail!(
                    "generation {}: error frame requires ok:false and an error shape",
                    generation.ordinal
                );
            }
        }
        _ => unreachable!("terminal position validated"),
    }
    Ok(())
}
