//! Turn stored content into the bytes and policy a share serves.
//!
//! Shared content is untrusted: an agent wrote it, and a stranger with the
//! link reads it. Two layers keep it contained:
//!
//! 1. **A frame we control.** HTML and SVG never become the top-level
//!    document. `/s/<token>` answers with a fixed shell page that carries no
//!    script and embeds the content in an `<iframe sandbox srcdoc>` without
//!    `allow-top-navigation` or `allow-popups`. Content can run its inline
//!    script but cannot navigate the page away (no phishing redirect), open
//!    windows, or read the link: its own address is `about:srcdoc` and the
//!    shell sends no referrer.
//! 2. **A policy that blocks the network.** The shell's CSP (inherited by
//!    the srcdoc frame) is `default-src 'none'` with `frame-src 'none'`, so
//!    content cannot fetch, load remote resources, submit forms, or navigate
//!    its frame to another site.
//!
//! Markdown renders server-side with raw HTML escaped, so it never runs
//! script and is served directly with a script-free policy. Every page shows
//! a banner saying who served it and when it expires.

use pulldown_cmark::{Event, Options, Parser, html};

use crate::store::ArtifactFormat;
use crate::util::rfc3339;

/// The shell around HTML and SVG: no script of its own, nothing loads or
/// leaves, and the embedded frame cannot navigate anywhere.
pub const CSP_SHELL: &str = "sandbox allow-scripts allow-forms allow-modals; default-src 'none'; \
     script-src 'unsafe-inline'; style-src 'unsafe-inline'; img-src data: blob:; \
     font-src data:; media-src data: blob:; connect-src 'none'; frame-src 'none'; \
     form-action 'none'; base-uri 'none'; frame-ancestors 'none'";

/// Rendered markdown and status pages: no script at all.
pub const CSP_STATIC: &str = "sandbox; default-src 'none'; style-src 'unsafe-inline'; \
     img-src data:; form-action 'none'; base-uri 'none'; frame-ancestors 'none'";

/// The passcode prompt: one form that posts back to the same link.
pub const CSP_PASSCODE: &str = "default-src 'none'; style-src 'unsafe-inline'; \
     form-action 'self'; base-uri 'none'; frame-ancestors 'none'";

/// `sandbox` tokens of the frame that holds shared HTML.
const HTML_FRAME_SANDBOX: &str = "allow-scripts allow-forms allow-modals";

/// What one share serves.
#[derive(Debug, PartialEq)]
pub struct Rendered {
    pub content_type: &'static str,
    pub csp: &'static str,
    pub body: String,
}

/// Render one version for a share that expires at `expires_at`.
pub fn render(format: ArtifactFormat, title: &str, content: &str, expires_at: i64) -> Rendered {
    let banner = banner(expires_at);
    match format {
        ArtifactFormat::Html => Rendered {
            content_type: "text/html; charset=utf-8",
            csp: CSP_SHELL,
            body: shell(title, &banner, HTML_FRAME_SANDBOX, content),
        },
        ArtifactFormat::Svg => Rendered {
            content_type: "text/html; charset=utf-8",
            csp: CSP_SHELL,
            body: shell(title, &banner, "", &svg_document(content)),
        },
        ArtifactFormat::Markdown => Rendered {
            content_type: "text/html; charset=utf-8",
            csp: CSP_STATIC,
            body: markdown_page(title, &banner, content),
        },
    }
}

/// A plain status page (404, 410, 429, 500) in the same visual frame.
pub fn status_page(heading: &str, detail: &str) -> String {
    page(
        heading,
        &format!(
            "<main><h1>{}</h1><p>{}</p></main>",
            escape_html(heading),
            escape_html(detail)
        ),
    )
}

/// The passcode prompt; `error` when the last attempt was wrong.
pub fn passcode_page(title: &str, error: Option<&str>) -> String {
    let error = error
        .map(|e| format!("<p class=\"err\" role=\"alert\">{}</p>", escape_html(e)))
        .unwrap_or_default();
    page(
        title,
        &format!(
            "<main class=\"narrow\"><h1>Protected link</h1>\
             <p>Enter the passcode you received with this link.</p>{error}\
             <form method=\"post\" autocomplete=\"off\">\
             <label for=\"passcode\">Passcode</label>\
             <input id=\"passcode\" name=\"passcode\" type=\"password\" required autofocus>\
             <button type=\"submit\">Open</button></form></main>"
        ),
    )
}

fn banner(expires_at: i64) -> String {
    format!(
        "<div class=\"iii-banner\" role=\"note\">Shared with iii · written by its author, not \
         reviewed by iii · link expires {}</div>",
        rfc3339(expires_at)
    )
}

/// The page that embeds untrusted content in a sandboxed srcdoc frame.
fn shell(title: &str, banner: &str, sandbox: &str, document: &str) -> String {
    format!(
        "<!doctype html>\n<html lang=\"en\"><head><meta charset=\"utf-8\">\
         <meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\
         <meta name=\"robots\" content=\"noindex,nofollow\">\
         <meta name=\"referrer\" content=\"no-referrer\">\
         <title>{title}</title><style>{SHELL_STYLE}</style></head>\
         <body>{banner}<iframe title=\"{title}\" sandbox=\"{sandbox}\" \
         referrerpolicy=\"no-referrer\" srcdoc=\"{doc}\"></iframe></body></html>\n",
        title = escape_html(title),
        doc = escape_html(document),
    )
}

fn svg_document(svg: &str) -> String {
    format!(
        "<!doctype html><html><head><style>html,body{{margin:0;height:100%}}\
         body{{display:grid;place-items:center}}svg{{max-width:100%;max-height:100%}}</style>\
         </head><body>{svg}</body></html>"
    )
}

/// Markdown to a standalone page. Raw HTML in the source is escaped and shown
/// as text: an RFC never needs markup the renderer did not produce.
fn markdown_page(title: &str, banner: &str, markdown: &str) -> String {
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
    page(
        title,
        &format!("{banner}<main><article>{body}</article></main>"),
    )
}

const SHELL_STYLE: &str = "\
html,body{margin:0;height:100%;background:#fff}
body{display:flex;flex-direction:column}
.iii-banner{flex:none;padding:6px 12px;font:12px/1.4 system-ui,sans-serif;color:#3f3f46;background:#f4f4f5;border-bottom:1px solid #e4e4e7}
iframe{flex:1;width:100%;border:0;display:block}";

const PAGE_STYLE: &str = "\
:root{color-scheme:light dark;--fg:#1d1d22;--muted:#5d5d68;--bg:#fbfaf7;--line:#e4e2dc;--code:#f1efe9;--err:#b42318}
@media(prefers-color-scheme:dark){:root{--fg:#ececf1;--muted:#a0a0ab;--bg:#111114;--line:#2a2a31;--code:#1b1b20;--err:#f97066}}
body{margin:0;background:var(--bg);color:var(--fg);font:16px/1.65 system-ui,-apple-system,Segoe UI,Roboto,sans-serif}
.iii-banner{padding:6px 12px;font-size:12px;color:var(--muted);border-bottom:1px solid var(--line)}
main{max-width:760px;margin:0 auto;padding:48px 24px 80px}main.narrow{max-width:420px}
h1,h2,h3{line-height:1.25}h1{font-size:2em}h2{margin-top:2em;padding-bottom:.3em;border-bottom:1px solid var(--line)}
a{color:inherit}code{background:var(--code);padding:.1em .35em;border-radius:4px;font-size:.9em}
pre{background:var(--code);padding:14px 16px;border-radius:6px;overflow:auto}pre code{background:none;padding:0}
blockquote{margin:0;padding-left:16px;border-left:3px solid var(--line);color:var(--muted)}
table{border-collapse:collapse;display:block;overflow:auto}th,td{border:1px solid var(--line);padding:6px 12px}
img{max-width:100%;height:auto}hr{border:0;border-top:1px solid var(--line)}
form{display:flex;flex-direction:column;gap:10px}label{font-size:14px;color:var(--muted)}
input{font:inherit;padding:10px 12px;border:1px solid var(--line);border-radius:6px;background:transparent;color:inherit}
button{font:inherit;padding:10px 12px;border:0;border-radius:6px;background:var(--fg);color:var(--bg);cursor:pointer}
.err{color:var(--err)}";

fn page(title: &str, inner: &str) -> String {
    format!(
        "<!doctype html>\n<html lang=\"en\"><head><meta charset=\"utf-8\">\
         <meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\
         <meta name=\"robots\" content=\"noindex,nofollow\">\
         <meta name=\"referrer\" content=\"no-referrer\">\
         <title>{}</title><style>{PAGE_STYLE}</style></head>\
         <body>{inner}</body></html>\n",
        escape_html(title)
    )
}

/// Escape text for an HTML text or attribute context.
pub fn escape_html(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + text.len() / 8);
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
    fn html_runs_inside_a_frame_that_cannot_navigate_away() {
        let out = render(
            ArtifactFormat::Html,
            "t",
            "<script>location='https://x'</script>",
            0,
        );
        assert_eq!(out.csp, CSP_SHELL);
        assert!(
            out.csp.contains("frame-src 'none'"),
            "the frame cannot be navigated off-site"
        );
        assert!(
            out.body
                .contains("sandbox=\"allow-scripts allow-forms allow-modals\"")
        );
        assert!(!out.body.contains("allow-top-navigation"));
        assert!(!out.body.contains("allow-popups"));
        assert!(!out.body.contains("allow-same-origin"));
        assert!(
            !out.body.contains("<script>"),
            "content is escaped into srcdoc"
        );
        assert!(
            out.body
                .contains("&lt;script&gt;location=&#39;https://x&#39;&lt;/script&gt;")
        );
    }

    #[test]
    fn the_shell_itself_carries_no_script_and_no_referrer() {
        let out = render(ArtifactFormat::Html, "t", "<p>x</p>", 0);
        let shell_markup = out.body.split("srcdoc=").next().expect("head part");
        assert!(!shell_markup.contains("<script"));
        assert!(
            out.body
                .contains("name=\"referrer\" content=\"no-referrer\"")
        );
        assert!(out.body.contains("Shared with iii"));
    }

    #[test]
    fn svg_is_framed_without_script() {
        let out = render(ArtifactFormat::Svg, "t", "<svg><script>x</script></svg>", 0);
        assert!(
            out.body.contains("sandbox=\"\""),
            "an empty sandbox runs no script"
        );
    }

    #[test]
    fn markdown_renders_and_escapes_raw_html() {
        let out = render(
            ArtifactFormat::Markdown,
            "RFC <1>",
            "# Title\n\n<script>alert(1)</script>\n\n| a |\n|---|\n| b |\n",
            0,
        );
        assert!(out.body.contains("<h1>Title</h1>"), "{}", out.body);
        assert!(out.body.contains("&lt;script&gt;"), "{}", out.body);
        assert!(!out.body.contains("<script>"), "{}", out.body);
        assert!(out.body.contains("<table>"));
        assert!(out.body.contains("<title>RFC &lt;1&gt;</title>"));
        assert_eq!(out.csp, CSP_STATIC);
    }

    #[test]
    fn static_pages_cannot_run_script() {
        assert!(!CSP_STATIC.contains("script-src"));
        assert!(CSP_STATIC.starts_with("sandbox;"));
        assert!(!CSP_PASSCODE.contains("script-src"));
    }

    #[test]
    fn the_passcode_page_escapes_its_error() {
        let body = passcode_page("t", Some("<b>wrong</b>"));
        assert!(body.contains("&lt;b&gt;wrong&lt;/b&gt;"));
        assert!(body.contains("method=\"post\""));
    }
}
