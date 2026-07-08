//! iii function ids ↔ OpenAI-style tool function names. `::` is not a valid
//! character in most chat-template tool-name grammars, so bus ids get
//! sanitized to `__` on the wire.

pub fn encode_tool_name(name: &str) -> String {
    name.replace("::", "__")
}

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
}
