//! Effective max_output_tokens precedence (spec § Output-token defaults).
///
/// `None` means "no budget to forward": only when nothing was requested or
/// configured, no soft cap is set, and the model's ceiling is unknown. The
/// provider then applies its own no-limit policy (omit the parameter, or
/// its configured/default value where the API requires one).
pub fn resolve_max_output_tokens(
    requested: Option<u64>,
    configured_max: Option<u64>, // operator's max_tokens in the config slice
    model_ceiling: Option<u64>,  // catalog ceiling when the model is known
    provider_default: u64,
    soft_cap: Option<u64>, // router-wide `output_token_max`; None = uncapped
) -> Option<u64> {
    if let Some(explicit) = requested.or(configured_max) {
        // A deliberate choice is honoured (no soft cap), clamped to the ceiling when known.
        return Some(model_ceiling.map_or(explicit, |c| explicit.min(c)));
    }
    match soft_cap {
        Some(cap) => Some(model_ceiling.map_or(provider_default, |c| c.min(cap))),
        // Uncapped: forward the model's own ceiling so no provider falls back
        // to a smaller default of its own.
        None => model_ceiling,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn precedence_per_spec_output_token_defaults() {
        let cap = Some(32_000);
        // explicit override wins, clamped down to the model ceiling
        assert_eq!(
            resolve_max_output_tokens(Some(100_000), None, Some(64_000), 8192, cap),
            Some(64_000)
        );
        assert_eq!(
            resolve_max_output_tokens(Some(4000), None, Some(64_000), 8192, cap),
            Some(4000)
        );
        // a deliberate choice is honoured above the soft cap
        assert_eq!(
            resolve_max_output_tokens(Some(50_000), None, Some(64_000), 8192, cap),
            Some(50_000)
        );
        // configured max_tokens is the explicit override when the request omits it
        assert_eq!(
            resolve_max_output_tokens(None, Some(50_000), Some(64_000), 8192, cap),
            Some(50_000)
        );
        assert_eq!(
            resolve_max_output_tokens(Some(1000), Some(50_000), Some(64_000), 8192, cap),
            Some(1000)
        );
        // known model without override: min(ceiling, soft cap)
        assert_eq!(
            resolve_max_output_tokens(None, None, Some(64_000), 8192, cap),
            Some(32_000)
        );
        assert_eq!(
            resolve_max_output_tokens(None, None, Some(16_000), 8192, cap),
            Some(16_000)
        );
        // unknown model: provider default; explicit passes through unclamped
        assert_eq!(
            resolve_max_output_tokens(None, None, None, 8192, cap),
            Some(8192)
        );
        assert_eq!(
            resolve_max_output_tokens(Some(99_000), None, None, 8192, cap),
            Some(99_000)
        );
    }

    #[test]
    fn no_soft_cap_forwards_the_model_ceiling_or_nothing() {
        // uncapped + known model: the model's own ceiling, never the provider default
        assert_eq!(
            resolve_max_output_tokens(None, None, Some(128_000), 8192, None),
            Some(128_000)
        );
        // uncapped + unknown model: nothing to forward; the provider decides
        assert_eq!(
            resolve_max_output_tokens(None, None, None, 8192, None),
            None
        );
        // explicit choices still apply without a soft cap
        assert_eq!(
            resolve_max_output_tokens(Some(200_000), None, Some(128_000), 8192, None),
            Some(128_000)
        );
        assert_eq!(
            resolve_max_output_tokens(None, Some(4000), Some(128_000), 8192, None),
            Some(4000)
        );
    }
}
