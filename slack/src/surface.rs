//! The registered function surface — the published interface. Kept as a flat
//! list so a test can assert the ids are unique and namespaced; richer
//! per-function golden schemas can be layered on later.

pub const FUNCTION_IDS: &[&str] = &[
    // chat
    "slack::chat::post-message",
    "slack::chat::update",
    "slack::chat::delete",
    "slack::chat::post-ephemeral",
    "slack::chat::schedule-message",
    "slack::chat::get-permalink",
    // conversations
    "slack::conversations::list",
    "slack::conversations::info",
    "slack::conversations::history",
    "slack::conversations::replies",
    "slack::conversations::create",
    "slack::conversations::invite",
    "slack::conversations::join",
    "slack::conversations::members",
    "slack::conversations::open",
    "slack::conversations::set-topic",
    "slack::conversations::set-purpose",
    "slack::conversations::archive",
    // reactions
    "slack::reactions::add",
    "slack::reactions::remove",
    "slack::reactions::get",
    // files
    "slack::files::upload",
    "slack::files::info",
    "slack::files::list",
    // users
    "slack::users::list",
    "slack::users::info",
    "slack::users::lookup-by-email",
    "slack::users::profile-get",
    // views
    "slack::views::open",
    "slack::views::publish",
    "slack::views::update",
    "slack::views::push",
    // pins / bookmarks
    "slack::pins::add",
    "slack::pins::list",
    "slack::bookmarks::add",
    // search
    "slack::search::messages",
    // assistant
    "slack::assistant::set-status",
    "slack::assistant::set-title",
    "slack::assistant::set-suggested-prompts",
    // admin / escape hatch
    "slack::auth::test",
    "slack::config-status",
    "slack::call",
];

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn ids_are_unique_and_namespaced() {
        let set: HashSet<&&str> = FUNCTION_IDS.iter().collect();
        assert_eq!(set.len(), FUNCTION_IDS.len(), "duplicate function id");
        for id in FUNCTION_IDS {
            assert!(id.starts_with("slack::"), "id not namespaced: {id}");
        }
    }
}
