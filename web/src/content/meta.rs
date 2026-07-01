//! Page-metadata extraction. Used as the BM25 query fallback when the caller
//! omits `query`.

/// Build a query string from `<title>` + `<meta name=description|keywords>`.
/// Returns "" when none are present or the HTML fails to parse.
pub fn extract_metadata_query(html: &str) -> String {
    let dom = match tl::parse(html, tl::ParserOptions::default()) {
        Ok(d) => d,
        Err(_) => return String::new(),
    };
    let parser = dom.parser();
    let mut parts: Vec<String> = Vec::new();
    if let Some(tag) = dom
        .query_selector("title")
        .and_then(|mut it| it.next())
        .and_then(|h| h.get(parser))
        .and_then(|n| n.as_tag())
    {
        parts.push(tag.inner_text(parser).trim().to_string());
    }
    if let Some(iter) = dom.query_selector("meta") {
        for handle in iter {
            let Some(tag) = handle.get(parser).and_then(|n| n.as_tag()) else {
                continue;
            };
            let name = tag
                .attributes()
                .get("name")
                .flatten()
                .map(|b| b.as_utf8_str().to_lowercase());
            if matches!(name.as_deref(), Some("description") | Some("keywords")) {
                if let Some(content) = tag.attributes().get("content").flatten() {
                    parts.push(content.as_utf8_str().trim().to_string());
                }
            }
        }
    }
    parts.retain(|p| !p.is_empty());
    parts.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pulls_title_and_meta() {
        let html = r#"<html><head><title>Rust Guide</title>
            <meta name="description" content="learn ownership">
            <meta name="keywords" content="borrow checker"></head><body>x</body></html>"#;
        let q = extract_metadata_query(html);
        assert!(q.contains("Rust Guide"));
        assert!(q.contains("ownership"));
        assert!(q.contains("borrow checker"));
    }

    #[test]
    fn empty_when_no_metadata() {
        assert_eq!(extract_metadata_query("<p>hi</p>"), "");
    }
}
