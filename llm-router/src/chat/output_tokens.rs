//! Effective max_output_tokens precedence (spec § Output-token defaults).
pub fn resolve_max_output_tokens(
    requested: Option<u64>,
    configured_max: Option<u64>, // operator's max_tokens in the config slice
    model_ceiling: Option<u64>,  // catalog ceiling when the model is known
    provider_default: u64,
    soft_cap: u64,
) -> u64 {
    if let Some(explicit) = requested.or(configured_max) {
        // A deliberate choice is honoured (no soft cap), clamped to the ceiling when known.
        return model_ceiling.map_or(explicit, |c| explicit.min(c));
    }
    model_ceiling.map_or(provider_default, |c| c.min(soft_cap))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn precedence_per_spec_output_token_defaults() {
        // explicit override wins, clamped down to the model ceiling
        assert_eq!(
            resolve_max_output_tokens(Some(100_000), None, Some(64_000), 8192, 32_000),
            64_000
        );
        assert_eq!(
            resolve_max_output_tokens(Some(4000), None, Some(64_000), 8192, 32_000),
            4000
        );
        // a deliberate choice is honoured above the soft cap
        assert_eq!(
            resolve_max_output_tokens(Some(50_000), None, Some(64_000), 8192, 32_000),
            50_000
        );
        // configured max_tokens is the explicit override when the request omits it
        assert_eq!(
            resolve_max_output_tokens(None, Some(50_000), Some(64_000), 8192, 32_000),
            50_000
        );
        assert_eq!(
            resolve_max_output_tokens(Some(1000), Some(50_000), Some(64_000), 8192, 32_000),
            1000
        );
        // known model without override: min(ceiling, soft cap)
        assert_eq!(
            resolve_max_output_tokens(None, None, Some(64_000), 8192, 32_000),
            32_000
        );
        assert_eq!(
            resolve_max_output_tokens(None, None, Some(16_000), 8192, 32_000),
            16_000
        );
        // unknown model: provider default; explicit passes through unclamped
        assert_eq!(
            resolve_max_output_tokens(None, None, None, 8192, 32_000),
            8192
        );
        assert_eq!(
            resolve_max_output_tokens(Some(99_000), None, None, 8192, 32_000),
            99_000
        );
    }
}
