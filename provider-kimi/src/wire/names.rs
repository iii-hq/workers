//! iii function ids ↔ OpenAI function names. OpenAI enforces
//! `^[a-zA-Z0-9_-]{1,64}$`; bus ids use `::` separators.

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
