//! Single-page content pipeline for `web::fetch`: content selection, content
//! filtering (pruning/BM25), and link/media extraction. Pure + synchronous;
//! the caller runs `process` inside `spawn_blocking`. Reused by later phases
//! (`web::extract`, `web::crawl`).

use serde::Serialize;

use crate::convert;
use crate::schemas::{ContentFilter, PageFormat};

pub mod filter;
pub mod links;
pub mod meta;
pub mod select;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Link {
    pub href: String,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Links {
    pub internal: Vec<Link>,
    pub external: Vec<Link>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MediaItem {
    pub src: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alt: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Media {
    pub images: Vec<MediaItem>,
    pub videos: Vec<MediaItem>,
    pub audios: Vec<MediaItem>,
}

/// Owned so the whole struct can move into `spawn_blocking`.
pub struct ContentOpts {
    pub format: PageFormat,
    pub content_filter: Option<ContentFilter>,
    pub target_elements: Vec<String>,
    pub excluded_tags: Vec<String>,
    pub include_links: bool,
    pub include_media: bool,
}

impl ContentOpts {
    /// True when any field activates the pipeline. When false, the caller must
    /// use the existing (backward-compatible) render path.
    pub fn is_enriched(&self) -> bool {
        self.content_filter.is_some()
            || !self.target_elements.is_empty()
            || !self.excluded_tags.is_empty()
            || self.include_links
            || self.include_media
    }
}

pub struct PageContent {
    /// `None` => caller keeps the raw body (size cap or over-depth; no transform ran).
    pub rendered: Option<String>,
    pub filtered: bool,
    pub links: Option<Links>,
    pub media: Option<Media>,
}

/// Run the single-page pipeline: exclude → scope → filter → render, plus
/// link/media extraction from the full document. Pure + synchronous; run under
/// `spawn_blocking`.
///
/// Link/media extraction is a flat, non-recursive scan, so it runs on every
/// input. The recursive transforms (markdown/text) and filtering run only when
/// `allow_transform` (the caller's within-size-cap decision) is true AND the
/// document isn't pathologically nested; otherwise `rendered` is `None` and the
/// caller keeps the raw body.
pub fn process(
    html: &str,
    base_url: &str,
    opts: &ContentOpts,
    allow_transform: bool,
) -> PageContent {
    // Flat, non-recursive (tl + query_selector); safe on any size/depth.
    let links = opts
        .include_links
        .then(|| links::extract_links(html, base_url));
    let media = opts
        .include_media
        .then(|| links::extract_media(html, base_url));

    if !allow_transform || convert::max_tag_depth(html) > convert::MAX_NESTING_DEPTH {
        return PageContent {
            rendered: None,
            filtered: false,
            links,
            media,
        };
    }

    let mut working = select::remove_excluded(html, &opts.excluded_tags);
    if let Some(scoped) = select::scope_to_targets(&working, &opts.target_elements) {
        working = scoped;
    }
    let mut filtered = false;
    if let Some(cf) = &opts.content_filter {
        let fallback = meta::extract_metadata_query(html);
        let f = filter::apply(&working, cf, &fallback);
        if !f.trim().is_empty() {
            working = f;
            filtered = true;
        }
    }

    let rendered = match opts.format {
        // On markdown failure, fall back to text of the PROCESSED html — never the
        // caller's unfiltered raw body, which would resurrect the removed boilerplate.
        PageFormat::Markdown => Some(
            convert::html_to_markdown(&working).unwrap_or_else(|_| convert::extract_text(&working)),
        ),
        PageFormat::Text => Some(convert::extract_text(&working)),
        PageFormat::Html => Some(working),
    };

    PageContent {
        rendered,
        filtered,
        links,
        media,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schemas::{ContentFilter, FilterType, PageFormat};

    fn opts() -> ContentOpts {
        ContentOpts {
            format: PageFormat::Markdown,
            content_filter: None,
            target_elements: Vec::new(),
            excluded_tags: Vec::new(),
            include_links: false,
            include_media: false,
        }
    }

    const PAGE: &str = r#"<body><nav class="nav"><a href="/h">Home</a></nav>
        <article><p>The quick brown fox jumps over the lazy dog through a long real paragraph.</p>
        <img src="/p.png" alt="pic"></article></body>"#;

    #[test]
    fn filter_replaces_rendered_with_fit_content() {
        let o = ContentOpts {
            content_filter: Some(ContentFilter {
                kind: FilterType::Pruning,
                query: None,
                threshold: None,
                threshold_type: None,
                min_word_threshold: None,
            }),
            ..opts()
        };
        let pc = process(PAGE, "https://s.test/", &o, true);
        let md = pc.rendered.unwrap();
        assert!(md.contains("quick brown fox"));
        assert!(!md.contains("Home"));
        assert!(pc.filtered);
    }

    #[test]
    fn links_and_media_extracted_when_requested() {
        let o = ContentOpts {
            include_links: true,
            include_media: true,
            ..opts()
        };
        let pc = process(PAGE, "https://s.test/", &o, true);
        assert_eq!(pc.links.unwrap().internal[0].href, "https://s.test/h");
        assert_eq!(pc.media.unwrap().images[0].src, "https://s.test/p.png");
    }

    #[test]
    fn extraction_runs_without_transform() {
        // Oversized body (allow_transform=false): no transform, but links/media
        // extraction still runs — the flat scan isn't gated by the size cap.
        let o = ContentOpts {
            include_links: true,
            include_media: true,
            ..opts()
        };
        let pc = process(PAGE, "https://s.test/", &o, false);
        assert!(pc.rendered.is_none());
        assert_eq!(pc.links.unwrap().internal[0].href, "https://s.test/h");
        assert_eq!(pc.media.unwrap().images[0].src, "https://s.test/p.png");
    }

    #[test]
    fn over_depth_skips_transform_but_still_extracts_links() {
        let deep = format!(
            "{}<a href=\"/deep\">D</a>{}",
            "<div>".repeat(300),
            "</div>".repeat(300)
        );
        let o = ContentOpts {
            include_links: true,
            include_media: true,
            ..opts()
        };
        let pc = process(&deep, "https://s.test/", &o, true);
        assert!(pc.rendered.is_none()); // recursive transform skipped on over-depth
        assert_eq!(pc.links.unwrap().internal[0].href, "https://s.test/deep"); // flat scan still runs
        assert!(pc.media.is_some());
    }
}
