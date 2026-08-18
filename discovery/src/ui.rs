//! Embedded console assets: the search_functions call card and the hook's
//! transcript line, registered through the shared console-ui plumbing.

use std::sync::Arc;

use iii_console_ui::ConsoleUi;
use iii_sdk::IIIClient;

const PAGE_JS: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/ui/dist/page.js"));
const STYLES_CSS: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/ui/dist/styles.css"));

pub fn register(iii: &Arc<IIIClient>) {
    ConsoleUi::new("discovery")
        .script("discovery/page.js", PAGE_JS)
        .style("discovery/styles.css", STYLES_CSS)
        .register(iii);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_page_registers_the_search_card_and_transcript_line() {
        assert!(PAGE_JS.contains("export"));
        assert!(PAGE_JS.contains("functionTriggers"));
        assert!(PAGE_JS.contains("discovery::search_functions"));
        assert!(PAGE_JS.contains("registerTranscriptRenderer"));
    }

    #[test]
    fn embedded_styles_stay_scoped_to_the_worker() {
        assert!(STYLES_CSS.contains("[data-iii-ui=discovery]"));
        assert!(!STYLES_CSS.contains("reflex"));
    }
}
