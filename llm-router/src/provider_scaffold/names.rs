//! iii function ids ↔ provider tool names. Upstream APIs enforce
//! `^[a-zA-Z0-9_-]{1,128}$`-style names; bus ids use `::` separators.

pub fn encode_tool_name(name: &str) -> String {
    name.replace("::", "__")
}

/// Inverse of `encode_tool_name`. Lossy precondition: an id containing a
/// literal `__` decodes to `::` — such ids are not in use on the bus today.
pub fn decode_tool_name(name: &str) -> String {
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
    fn literal_double_underscore_is_the_accepted_lossy_case() {
        // known limitation: a literal `__` in an id decodes to `::`
        assert_eq!(decode_tool_name("a__b"), "a::b");
    }
}
