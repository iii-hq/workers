//! iii function ids ↔ OpenAI function names. OpenAI enforces
//! `^[a-zA-Z0-9_-]{1,64}$`; bus ids use `::` separators.
//!
//! One deliberate alias: `coder::apply-patch` travels as `apply_patch` —
//! the EXACT tool name codex models are trained on (it is also emitted as
//! a freeform `custom` tool; see `wire::tools` and the sse custom-input
//! handling).

/// The wire name codex models know for the V4A patch tool.
pub const APPLY_PATCH_WIRE: &str = "apply_patch";
/// The bus function the alias maps to.
pub const APPLY_PATCH_FN: &str = "coder::apply-patch";

pub fn encode_tool_name(name: &str) -> String {
    if name == APPLY_PATCH_FN {
        return APPLY_PATCH_WIRE.to_string();
    }
    name.replace("::", "__")
}

pub fn decode_tool_name(name: &str) -> String {
    if name == APPLY_PATCH_WIRE {
        return APPLY_PATCH_FN.to_string();
    }
    name.replace("__", "::")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_bus_ids() {
        assert_eq!(encode_tool_name("web::fetch"), "web__fetch");
        assert_eq!(decode_tool_name("web__fetch"), "web::fetch");
        assert_eq!(decode_tool_name(&encode_tool_name("a::b::c")), "a::b::c");
    }

    #[test]
    fn plain_names_pass_through() {
        assert_eq!(encode_tool_name("submit_result"), "submit_result");
        assert_eq!(decode_tool_name("submit_result"), "submit_result");
    }

    #[test]
    fn apply_patch_aliases_both_directions() {
        assert_eq!(encode_tool_name("coder::apply-patch"), "apply_patch");
        assert_eq!(decode_tool_name("apply_patch"), "coder::apply-patch");
        // The generic codec must not double-map the alias.
        assert_eq!(
            decode_tool_name(&encode_tool_name("coder::apply-patch")),
            "coder::apply-patch"
        );
    }
}
