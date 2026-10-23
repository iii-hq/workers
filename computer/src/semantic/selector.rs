//! Selector validation. xa11y's grammar is reused as is; what it lets through
//! silently is refused here: an unknown role word becomes a platform-role
//! match that finds nothing, and an unknown attribute name is resolved against
//! the element's `raw` map — which still holds the unredacted value of a
//! password field, so it would be a prefix oracle.

use xa11y_core::selector::{MatchOp, RoleMatch};
use xa11y_core::{Role, SelectorGroup};

use super::{Code, SemanticError, MAX_SELECTOR_BYTES};

/// Attribute names a selector may filter on: xa11y's normalized names minus
/// `stable_id` and `bounds` (never shown to the agent).
pub const ALLOWED_ATTRS: &[&str] = &[
    "role",
    "name",
    "value",
    "description",
    "enabled",
    "visible",
    "focused",
    "active",
    "focusable",
    "selected",
    "editable",
    "modal",
    "required",
    "busy",
    "expanded",
    "checked",
    "minimized",
    "maximized",
    "fullscreen",
];

const ROLE_HINT: &str = "roles are snake_case, e.g. button, text_field, text_area, check_box, \
                         radio_button, list, list_item, menu_item, tab, link, window, dialog, group";

pub fn parse_selector(input: &str) -> Result<SelectorGroup, SemanticError> {
    if input.len() > MAX_SELECTOR_BYTES {
        return Err(SemanticError::new(
            Code::InvalidSelector,
            format!("selector is longer than {MAX_SELECTOR_BYTES} bytes"),
        ));
    }
    let group = SelectorGroup::parse(input)
        .map_err(|e| SemanticError::new(Code::InvalidSelector, e.to_string()))?;
    for clause in &group.clauses {
        for segment in &clause.segments {
            if let Some(RoleMatch::Platform(word)) = &segment.simple.role {
                return Err(SemanticError::new(
                    Code::InvalidSelector,
                    format!("unknown role '{word}' ({ROLE_HINT})"),
                ));
            }
            for filter in &segment.simple.filters {
                if !ALLOWED_ATTRS.contains(&filter.attr.as_str()) {
                    return Err(SemanticError::new(
                        Code::InvalidSelector,
                        format!(
                            "attribute '{}' is not allowed; use one of: {}",
                            filter.attr,
                            ALLOWED_ATTRS.join(", ")
                        ),
                    ));
                }
                if filter.attr == "role"
                    && (filter.op != MatchOp::Exact
                        || Role::from_snake_case(&filter.value).is_none())
                {
                    return Err(SemanticError::new(
                        Code::InvalidSelector,
                        format!(
                            "role filters need `=` and a known role, got [role ... '{}'] ({ROLE_HINT})",
                            filter.value
                        ),
                    ));
                }
            }
        }
    }
    Ok(group)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::semantic::Code;

    fn code(s: &str) -> Option<Code> {
        parse_selector(s).err().map(|e| e.code)
    }

    #[test]
    fn accepts_roles_states_groups_and_nth() {
        for ok in [
            "button[name='Salvar']",
            "text_field[name='Nome'][value='ação, João, €']",
            "check_box[checked='on']",
            "[enabled='true'][name^='OK']",
            "dialog button[name='OK']",
            "list > list_item:nth(2)",
            "button[name='OK'], link[name='OK']",
            "*[role='button']",
        ] {
            assert!(parse_selector(ok).is_ok(), "{ok}");
        }
    }

    #[test]
    fn unknown_roles_are_refused_instead_of_matching_nothing() {
        assert_eq!(code("checkbox[name='x']"), Some(Code::InvalidSelector));
        assert_eq!(code("[role='buton']"), Some(Code::InvalidSelector));
        assert_eq!(code("[role^='but']"), Some(Code::InvalidSelector));
        let msg = parse_selector("buton").unwrap_err().message;
        assert!(msg.contains("unknown role 'buton'"), "{msg}");
    }

    #[test]
    fn raw_platform_attributes_are_refused_in_every_clause() {
        // These resolve against `raw`, which keeps unredacted values.
        assert_eq!(
            code("text_field[AXValue^='a']"),
            Some(Code::InvalidSelector)
        );
        assert_eq!(code("[uia_value^='a']"), Some(Code::InvalidSelector));
        assert_eq!(code("[stable_id='x']"), Some(Code::InvalidSelector));
        assert_eq!(
            code("button[name='OK'], [bounds*='1']"),
            Some(Code::InvalidSelector)
        );
        // AT-SPI keeps its raw value and role too.
        assert_eq!(code("[atspi_value^='a']"), Some(Code::InvalidSelector));
        assert_eq!(
            code("text_field[atspi_role='password text']"),
            Some(Code::InvalidSelector)
        );
        let msg = parse_selector("[uia_value='a']").unwrap_err().message;
        assert!(
            msg.contains("name, value"),
            "lists the allowed names: {msg}"
        );
    }

    #[test]
    fn syntax_errors_and_oversize_are_invalid_selector() {
        assert_eq!(code(""), Some(Code::InvalidSelector));
        assert_eq!(code("button[name='x'"), Some(Code::InvalidSelector));
        let long = format!("button[name='{}']", "a".repeat(1_100));
        assert_eq!(code(&long), Some(Code::InvalidSelector));
    }
}
