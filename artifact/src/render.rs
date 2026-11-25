//! Turn stored content into the bytes and policy a share serves.
//!
//! Shared content is untrusted: an agent wrote it, and a stranger with the
//! link reads it. Every format is therefore served under a CSP `sandbox`
//! (an opaque origin with no cookies or storage of its own) and
//! `default-src 'none'`, so it can neither phone home nor load anything from
//! the network. Only HTML gets to run its own inline script, which is what a
//! clickable prototype needs.

use pulldown_cmark::{Event, Options, Parser, html};

use crate::store::ArtifactFormat;

/// HTML prototypes: inline script and style run, nothing loads or leaves.
/// Forms may run their JS handlers but `form-action 'none'` stops any submit.
pub const CSP_HTML: &str = "sandbox allow-scripts allow-forms allow-modals; default-src 'none'; \
     script-src 'unsafe-inline'; style-src 'unsafe-inline'; img-src data: blob:; \
     font-src data:; media-src data: blob:; connect-src 'none'; form-action 'none'; \
     base-uri 'none'; frame-ancestors 'none'";

/// Rendered markdown and SVG: no script at all.
pub const CSP_STATIC: &str = "sandbox; default-src 'none'; style-src 'unsafe-inline'; \
     img-src data:; form-action 'none'; base-uri 'none'; frame-ancestors 'none'";

/// What one share serves.
#[derive(Debug, PartialEq)]
pub struct Rendered {
    pub content_type: &'static str,
    pub csp: &'static str,
    pub body: String,
}

/// Render one version for a share.
pub fn render(format: ArtifactFormat, title: &str, content: &str) -> Rendered {
    match format {
        ArtifactFormat::Html => Rendered {
            content_type: "text/html; charset=utf-8",
            csp: CSP_HTML,
            body: content.to_string(),
        },
        ArtifactFormat::Markdown => Rendered {
            content_type: "text/html; charset=utf-8",
            csp: CSP_STATIC,
            body: markdown_page(title, content),
        },
        ArtifactFormat::Svg => Rendered {
            content_type: "image/svg+xml",
            csp: CSP_STATIC,
            body: content.to_string(),
        },
    }
}

/// A plain status page (404, 410, 500) in the same visual frame.
pub fn status_page(heading: &str, detail: &str) -> String {
    page(
        heading,
        &format!(
            "<h1>{}</h1><p>{}</p>",
            escape_html(heading),
            escape_html(detail)
        ),
    )
}

/// Markdown to a standalone page. Raw HTML in the source is escaped and shown
/// as text: an RFC never needs markup the renderer did not produce.
fn markdown_page(title: &str, markdown: &str) -> String {
    let options = Options::ENABLE_TABLES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_HEADING_ATTRIBUTES;
    let parser = Parser::new_ext(markdown, options).map(|event| match event {
        Event::Html(raw) | Event::InlineHtml(raw) => Event::Text(raw),
        other => other,
    });
    let mut body = String::with_capacity(markdown.len() * 3 / 2);
    html::push_html(&mut body, parser);
    page(title, &format!("<article>{body}</article>"))
}

const PAGE_STYLE: &str = "\
:root{color-scheme:light dark;--fg:#1d1d22;--muted:#5d5d68;--bg:#fbfaf7;--line:#e4e2dc;--code:#f1efe9}
@media(prefers-color-scheme:dark){:root{--fg:#ececf1;--muted:#a0a0ab;--bg:#111114;--line:#2a2a31;--code:#1b1b20}}
body{margin:0;background:var(--bg);color:var(--fg);font:16px/1.65 system-ui,-apple-system,Segoe UI,Roboto,sans-serif}
main{max-width:760px;margin:0 auto;padding:48px 24px 80px}
h1,h2,h3{line-height:1.25}h1{font-size:2em}h2{margin-top:2em;padding-bottom:.3em;border-bottom:1px solid var(--line)}
a{color:inherit}code{background:var(--code);padding:.1em .35em;border-radius:4px;font-size:.9em}
pre{background:var(--code);padding:14px 16px;border-radius:6px;overflow:auto}pre code{background:none;padding:0}
blockquote{margin:0;padding-left:16px;border-left:3px solid var(--line);color:var(--muted)}
table{border-collapse:collapse;display:block;overflow:auto}th,td{border:1px solid var(--line);padding:6px 12px}
img{max-width:100%}hr{border:0;border-top:1px solid var(--line)}";

fn page(title: &str, inner: &str) -> String {
    format!(
        "<!doctype html>\n<html lang=\"en\"><head><meta charset=\"utf-8\">\
         <meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\
         <meta name=\"robots\" content=\"noindex,nofollow\">\
         <title>{}</title><style>{PAGE_STYLE}</style></head>\
         <body><main>{inner}</main></body></html>\n",
        escape_html(title)
    )
}

/// Escape text for an HTML text or attribute context.
pub fn escape_html(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            other => out.push(other),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn html_is_served_verbatim_under_the_script_sandbox() {
        let out = render(ArtifactFormat::Html, "t", "<h1>hi</h1>");
        assert_eq!(out.body, "<h1>hi</h1>");
        assert!(out.csp.starts_with("sandbox allow-scripts"));
        assert!(out.csp.contains("connect-src 'none'"));
    }

    #[test]
    fn markdown_renders_and_escapes_raw_html() {
        let out = render(
            ArtifactFormat::Markdown,
            "RFC <1>",
            "# Title\n\n<script>alert(1)</script>\n\n| a |\n|---|\n| b |\n",
        );
        assert!(out.body.contains("<h1>Title</h1>"), "{}", out.body);
        assert!(out.body.contains("&lt;script&gt;"), "{}", out.body);
        assert!(!out.body.contains("<script>"), "{}", out.body);
        assert!(out.body.contains("<table>"));
        assert!(out.body.contains("<title>RFC &lt;1&gt;</title>"));
        assert_eq!(out.csp, CSP_STATIC);
    }

    #[test]
    fn static_formats_cannot_run_script() {
        assert!(!CSP_STATIC.contains("script-src"));
        assert!(CSP_STATIC.starts_with("sandbox;"));
        let svg = render(ArtifactFormat::Svg, "t", "<svg/>");
        assert_eq!(svg.content_type, "image/svg+xml");
        assert_eq!(svg.csp, CSP_STATIC);
    }

    #[test]
    fn status_pages_escape_their_text() {
        let body = status_page("Gone", "<b>expired</b>");
        assert!(body.contains("&lt;b&gt;expired&lt;/b&gt;"));
    }
}
