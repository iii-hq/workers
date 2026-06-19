//! Convert LLM markdown to Telegram-compatible HTML.

use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, LinkType, Options, Parser, Tag, TagEnd};

const HTML_PARSE_MODE: &str = "HTML";

/// Format outgoing assistant text as Telegram HTML (markdown converted).
pub fn format_outgoing(text: &str) -> (String, Option<&'static str>) {
    (markdown_to_html(text), Some(HTML_PARSE_MODE))
}

fn escape_html(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _ => out.push(ch),
        }
    }
    out
}

fn escape_attr(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            '<' => out.push_str("&lt;"),
            _ => out.push(ch),
        }
    }
    out
}

fn markdown_to_html(text: &str) -> String {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TABLES);

    let parser = Parser::new_ext(text, options);
    let mut out = String::new();
    let mut list_stack: Vec<ListState> = Vec::new();
    let mut table_row_first = true;

    for event in parser {
        match event {
            Event::Start(tag) => match tag {
                Tag::Paragraph => {}
                Tag::Heading { .. } => out.push_str("<b>"),
                Tag::BlockQuote(_) => out.push_str("<blockquote>"),
                Tag::CodeBlock(kind) => match kind {
                    CodeBlockKind::Fenced(info) => {
                        out.push_str("<pre><code");
                        if !info.is_empty() {
                            out.push_str(" class=\"language-");
                            out.push_str(&escape_attr(&info));
                            out.push('"');
                        }
                        out.push('>');
                    }
                    CodeBlockKind::Indented => out.push_str("<pre><code>"),
                },
                Tag::List(start) => {
                    list_stack.push(ListState {
                        ordered: start.is_some(),
                        next: start.unwrap_or(1),
                    });
                }
                Tag::Item => {
                    if let Some(list) = list_stack.last_mut() {
                        if list.ordered {
                            out.push_str(&format!("{}. ", list.next));
                            list.next += 1;
                        } else {
                            out.push_str("- ");
                        }
                    }
                }
                Tag::Emphasis => out.push_str("<i>"),
                Tag::Strong => out.push_str("<b>"),
                Tag::Strikethrough => out.push_str("<s>"),
                Tag::Link {
                    link_type: LinkType::Email | LinkType::Autolink,
                    dest_url,
                    ..
                }
                | Tag::Link {
                    link_type: LinkType::Inline,
                    dest_url,
                    ..
                } => {
                    out.push_str("<a href=\"");
                    out.push_str(&escape_attr(&dest_url));
                    out.push_str("\">");
                }
                Tag::Table(_) => {}
                Tag::TableHead | Tag::TableRow => {
                    table_row_first = true;
                }
                Tag::TableCell => {
                    if !table_row_first {
                        out.push_str(" | ");
                    }
                    table_row_first = false;
                }
                Tag::Image { dest_url, .. } => {
                    out.push_str("<a href=\"");
                    out.push_str(&escape_attr(&dest_url));
                    out.push_str("\">");
                }
                _ => {}
            },
            Event::End(tag_end) => match tag_end {
                TagEnd::Paragraph => out.push_str("\n\n"),
                TagEnd::Heading(level) => {
                    out.push_str("</b>");
                    if level != HeadingLevel::H6 {
                        out.push('\n');
                    }
                    out.push('\n');
                }
                TagEnd::BlockQuote(_) => out.push_str("</blockquote>\n"),
                TagEnd::CodeBlock => out.push_str("</code></pre>\n\n"),
                TagEnd::List(_) => {
                    list_stack.pop();
                    out.push('\n');
                }
                TagEnd::Item => out.push('\n'),
                TagEnd::Emphasis => out.push_str("</i>"),
                TagEnd::Strong => out.push_str("</b>"),
                TagEnd::Strikethrough => out.push_str("</s>"),
                TagEnd::Link => out.push_str("</a>"),
                TagEnd::Table => {
                    out.push('\n');
                }
                TagEnd::TableHead | TagEnd::TableRow => out.push('\n'),
                TagEnd::TableCell => {}
                TagEnd::Image => out.push_str("</a>"),
                _ => {}
            },
            Event::Text(text) => {
                out.push_str(&escape_html(&text));
            }
            Event::Code(text) => {
                out.push_str("<code>");
                out.push_str(&escape_html(&text));
                out.push_str("</code>");
            }
            Event::Html(html) => out.push_str(&escape_html(&html)),
            Event::InlineHtml(html) => out.push_str(&escape_html(&html)),
            Event::SoftBreak | Event::HardBreak => out.push('\n'),
            Event::Rule => out.push_str("\n──────────\n\n"),
            Event::TaskListMarker(checked) => {
                if checked {
                    out.push_str("[x] ");
                } else {
                    out.push_str("[ ] ");
                }
            }
            Event::FootnoteReference(_) => {}
            Event::InlineMath(_) | Event::DisplayMath(_) => {}
        }
    }

    trim_trailing_whitespace(&out)
}

fn trim_trailing_whitespace(text: &str) -> String {
    text.trim_end().to_string()
}

#[derive(Debug, Clone, Copy)]
struct ListState {
    ordered: bool,
    next: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bold_to_html() {
        let (text, mode) = format_outgoing("**What I CAN do:**");
        assert_eq!(text, "<b>What I CAN do:</b>");
        assert_eq!(mode, Some("HTML"));
    }

    #[test]
    fn markdown_bold_to_html() {
        let (text, mode) = format_outgoing("**bold**");
        assert_eq!(text, "<b>bold</b>");
        assert_eq!(mode, Some("HTML"));
    }

    #[test]
    fn inline_code() {
        assert_eq!(
            markdown_to_html("Use `engine::functions::list` here."),
            "Use <code>engine::functions::list</code> here."
        );
    }

    #[test]
    fn fenced_code_block() {
        let md = "```python\nprint('hi')\n```";
        assert_eq!(
            markdown_to_html(md),
            "<pre><code class=\"language-python\">print('hi')\n</code></pre>"
        );
    }

    #[test]
    fn link() {
        assert_eq!(
            markdown_to_html("[docs](https://example.com)"),
            "<a href=\"https://example.com\">docs</a>"
        );
    }

    #[test]
    fn escapes_special_chars() {
        assert_eq!(markdown_to_html("2 < 3 & 4 > 1"), "2 &lt; 3 &amp; 4 &gt; 1");
    }

    #[test]
    fn heading_renders_bold() {
        assert_eq!(markdown_to_html("# Title"), "<b>Title</b>");
    }

    #[test]
    fn unordered_list() {
        assert_eq!(markdown_to_html("- one\n- two"), "- one\n- two");
    }

    #[test]
    fn ordered_list() {
        assert_eq!(
            markdown_to_html("1. first\n2. second"),
            "1. first\n2. second"
        );
    }

    #[test]
    fn edit_throttle_uses_entry_revision_key() {
        use crate::deps::RuntimeState;
        use crate::render::throttle;

        let rt = RuntimeState::new();
        // Main channel consumes revision 5 for entry "e1".
        assert!(throttle::should_edit(&rt, 1, 10, 5, "s1", "e1", 0));
        // A different entry id gets its own throttle slot.
        assert!(throttle::should_edit(&rt, 1, 10, 5, "s1", "e2", 0));
    }
}
