//! Content transforms for `web::fetch` page-reading mode. Pure functions,
//! no I/O. HTML→Markdown via `htmd`; plain-text via an iterative walk over
//! `tl`'s flat node arena that skips non-content subtrees. Callers MUST run
//! `max_tag_depth` and skip the transform when it exceeds `MAX_NESTING_DEPTH`
//! (htmd recurses over an rcdom tree; a stack overflow ABORTS the process
//! and is not catchable by `catch_unwind`).

use std::panic::{catch_unwind, AssertUnwindSafe};

use crate::schemas::PageFormat;

pub const BROWSER_USER_AGENT: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/143.0.0.0 Safari/537.36";
pub const ACCEPT_LANGUAGE: &str = "en-US,en;q=0.9";
pub const MAX_NESTING_DEPTH: usize = 200;

const SKIP_TAGS: [&str; 6] = ["script", "style", "noscript", "iframe", "object", "embed"];
const BLOCK_TAGS: [&str; 19] = [
    "p", "div", "h1", "h2", "h3", "h4", "h5", "h6", "li", "tr", "section", "article", "header",
    "footer", "blockquote", "pre", "table", "ul", "ol",
];

pub fn accept_header_for(format: PageFormat) -> &'static str {
    match format {
        PageFormat::Markdown => {
            "text/markdown;q=1.0, text/x-markdown;q=0.9, text/plain;q=0.8, text/html;q=0.7, */*;q=0.1"
        }
        PageFormat::Text => "text/plain;q=1.0, text/markdown;q=0.9, text/html;q=0.8, */*;q=0.1",
        PageFormat::Html => {
            "text/html;q=1.0, application/xhtml+xml;q=0.9, text/plain;q=0.8, text/markdown;q=0.7, */*;q=0.1"
        }
    }
}

pub fn is_image_mime(mime: &str) -> bool {
    mime.starts_with("image/")
}

pub fn is_viewable_image_mime(mime: &str) -> bool {
    matches!(mime, "image/jpeg" | "image/png" | "image/gif" | "image/webp")
}

/// Maximum tag-nesting depth, computed iteratively (no recursion, cannot
/// itself overflow). Used as a pre-transform guard.
pub fn max_tag_depth(html: &str) -> usize {
    let dom = match tl::parse(html, tl::ParserOptions::default()) {
        Ok(d) => d,
        Err(_) => return 0,
    };
    let parser = dom.parser();
    let mut max = 0usize;
    // stack of (node_handle, depth)
    let mut stack: Vec<(tl::NodeHandle, usize)> =
        dom.children().iter().map(|h| (*h, 1usize)).collect();
    while let Some((handle, depth)) = stack.pop() {
        if depth > max {
            max = depth;
        }
        if let Some(tag) = handle.get(parser).and_then(|n| n.as_tag()) {
            for child in tag.children().top().as_slice().iter() {
                stack.push((*child, depth + 1));
            }
        }
    }
    max
}

/// Iterative plain-text extraction. Skips SKIP_TAGS subtrees entirely (by
/// not descending), emits '\n' for <br> and on BLOCK_TAGS close, collapses
/// 3+ newlines to 2, trims.
pub fn extract_text(html: &str) -> String {
    enum Work {
        Enter(tl::NodeHandle),
        CloseBlock,
    }
    let dom = match tl::parse(html, tl::ParserOptions::default()) {
        Ok(d) => d,
        Err(_) => return String::new(),
    };
    let parser = dom.parser();
    let mut out = String::new();
    let mut stack: Vec<Work> = dom.children().iter().rev().map(|h| Work::Enter(*h)).collect();

    while let Some(item) = stack.pop() {
        match item {
            Work::CloseBlock => out.push('\n'),
            Work::Enter(handle) => {
                let Some(node) = handle.get(parser) else { continue };
                match node {
                    tl::Node::Raw(bytes) => out.push_str(&bytes.as_utf8_str()),
                    tl::Node::Tag(tag) => {
                        let name = tag.name().as_utf8_str();
                        let name = name.as_ref();
                        if SKIP_TAGS.contains(&name) {
                            continue; // skip subtree
                        }
                        if name == "br" {
                            out.push('\n');
                            continue;
                        }
                        if BLOCK_TAGS.contains(&name) {
                            stack.push(Work::CloseBlock);
                        }
                        for child in tag.children().top().as_slice().iter().rev() {
                            stack.push(Work::Enter(*child));
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    collapse_blank_lines(&out).trim().to_string()
}

fn collapse_blank_lines(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut newlines = 0usize;
    for ch in s.chars() {
        if ch == '\n' {
            newlines += 1;
            if newlines <= 2 {
                out.push('\n');
            }
        } else {
            newlines = 0;
            out.push(ch);
        }
    }
    out
}

/// HTML→Markdown via htmd. Returns Err on failure so the caller falls back
/// to the raw body. Wrapped in catch_unwind for ordinary panics; the
/// caller's depth guard handles the (uncatchable) stack-overflow case.
pub fn html_to_markdown(html: &str) -> Result<String, String> {
    let result = catch_unwind(AssertUnwindSafe(|| {
        htmd::HtmlToMarkdown::builder()
            .skip_tags(vec!["script", "style", "meta", "link"])
            .build()
            .convert(html)
    }));
    match result {
        Ok(Ok(md)) => Ok(md),
        Ok(Err(e)) => Err(format!("htmd convert failed: {e}")),
        Err(_) => Err("htmd panicked".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schemas::PageFormat;

    #[test]
    fn mime_helpers() {
        assert!(is_image_mime("image/png"));
        assert!(!is_image_mime("text/html"));
        assert!(is_viewable_image_mime("image/jpeg"));
        assert!(!is_viewable_image_mime("image/svg+xml"));
    }

    #[test]
    fn accept_headers_differ_by_format() {
        assert!(accept_header_for(PageFormat::Markdown).contains("text/markdown"));
        assert!(accept_header_for(PageFormat::Text).starts_with("text/plain"));
        assert!(accept_header_for(PageFormat::Html).contains("application/xhtml+xml"));
    }

    #[test]
    fn extracts_text_skipping_script_and_blocking() {
        let html = "<html><body><h1>Title</h1><script>var x=1;</script><p>Hello</p><p>World</p></body></html>";
        let text = extract_text(html);
        assert!(text.contains("Title"));
        assert!(text.contains("Hello"));
        assert!(text.contains("World"));
        assert!(!text.contains("var x"));
        // block tags force separation — no "TitleHello" run
        assert!(!text.contains("TitleHello"));
    }

    #[test]
    fn extract_text_collapses_blank_runs_and_trims() {
        let html = "<div></div><div></div><div></div><p>only</p>";
        let text = extract_text(html);
        assert!(!text.contains("\n\n\n"));
        assert_eq!(text.trim(), text);
    }

    #[test]
    fn br_becomes_newline() {
        let text = extract_text("a<br>b");
        assert!(text.contains("a\nb"));
    }

    #[test]
    fn markdown_drops_script_style_and_renders_heading() {
        let md = html_to_markdown("<h1>Hi</h1><script>bad()</script><style>x{}</style>").unwrap();
        assert!(md.contains("# Hi") || md.contains("Hi"));
        assert!(!md.contains("bad()"));
        assert!(!md.contains("x{}"));
    }

    #[test]
    fn depth_guard_counts_nesting() {
        let deep = "<div>".repeat(50) + "x" + &"</div>".repeat(50);
        assert!(max_tag_depth(&deep) >= 50);
        assert!(max_tag_depth("<p>flat</p>") <= 2);
    }
}
