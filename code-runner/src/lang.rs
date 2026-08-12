use serde::{Deserialize, Serialize};

/// Which engine backs a runtime.
///
/// Only `node` and `python` are accepted; anything else fails deserialization
/// before a handler runs, so the schema itself is the validation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Lang {
    Node,
    Python,
}

impl std::fmt::Display for Lang {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Node => "node",
            Self::Python => "python",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_two_known_languages_deserialize() {
        assert_eq!(
            serde_json::from_str::<Lang>("\"node\"").unwrap(),
            Lang::Node
        );
        assert_eq!(
            serde_json::from_str::<Lang>("\"python\"").unwrap(),
            Lang::Python
        );
        assert!(serde_json::from_str::<Lang>("\"ruby\"").is_err());
        // The wire spelling is lowercase; `Display` must agree with it, since
        // error messages name the language back to the caller.
        assert_eq!(Lang::Node.to_string(), "node");
        assert_eq!(Lang::Python.to_string(), "python");
    }
}
