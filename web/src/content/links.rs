//! Link & media extraction from the full document. Relative URLs resolve
//! against the document's `<base href>` when present (per the HTML spec),
//! otherwise the final (post-redirect) fetch URL. Links are classified
//! internal/external by comparing against the fetch host.

use tl::{parse, ParserOptions};
use url::Url;

use crate::content::{Link, Links, Media, MediaItem};

fn resolve(base: &Option<Url>, raw: &str) -> String {
    match base {
        Some(b) => b
            .join(raw)
            .map(|u| u.to_string())
            .unwrap_or_else(|_| raw.to_string()),
        None => raw.to_string(),
    }
}

/// Effective base URL for resolving relative references: the first `<base href>`
/// in the document (itself resolved against the fetch URL, since it may be
/// relative), falling back to the fetch URL.
fn effective_base(dom: &tl::VDom, fetch_url: &str) -> Option<Url> {
    let fetch = Url::parse(fetch_url).ok();
    let parser = dom.parser();
    if let Some(iter) = dom.query_selector("base") {
        for handle in iter {
            let Some(tag) = handle.get(parser).and_then(|n| n.as_tag()) else {
                continue;
            };
            if let Some(href) = tag.attributes().get("href").flatten() {
                let href = href.as_utf8_str();
                let href = href.trim();
                if !href.is_empty() {
                    return match &fetch {
                        Some(f) => f.join(href).ok().or_else(|| fetch.clone()),
                        None => Url::parse(href).ok(),
                    };
                }
            }
        }
    }
    fetch
}

/// `src` of the first `<source>` child of a `<video>`/`<audio>` element — the
/// standard `<video><source src=…></video>` markup where the element itself has
/// no `src`.
fn source_child_src(tag: &tl::HTMLTag, parser: &tl::Parser) -> Option<String> {
    for child in tag.children().top().as_slice() {
        let Some(ct) = child.get(parser).and_then(|n| n.as_tag()) else {
            continue;
        };
        if ct.name().as_utf8_str().eq_ignore_ascii_case("source") {
            if let Some(src) = ct.attributes().get("src").flatten() {
                let s = src.as_utf8_str();
                if !s.trim().is_empty() {
                    return Some(s.into_owned());
                }
            }
        }
    }
    None
}

pub fn extract_links(html: &str, base: &str) -> Links {
    let mut links = Links {
        internal: Vec::new(),
        external: Vec::new(),
    };
    let Ok(dom) = parse(html, ParserOptions::default()) else {
        return links;
    };
    let parser = dom.parser();
    let resolve_base = effective_base(&dom, base);
    // Classify against the FETCH host (not the <base> host): a <base> pointing
    // off-site still yields links external to the page we actually fetched.
    let fetch_host = Url::parse(base)
        .ok()
        .and_then(|u| u.host_str().map(str::to_string));

    if let Some(iter) = dom.query_selector("a") {
        for handle in iter {
            let Some(tag) = handle.get(parser).and_then(|n| n.as_tag()) else {
                continue;
            };
            let Some(href) = tag.attributes().get("href").flatten() else {
                continue;
            };
            let href = href.as_utf8_str();
            if href.trim().is_empty() {
                continue;
            }
            let abs = resolve(&resolve_base, &href);
            let host = Url::parse(&abs)
                .ok()
                .and_then(|u| u.host_str().map(str::to_string));
            let is_internal = match (&fetch_host, &host) {
                (Some(b), Some(h)) => b == h,
                // No comparable host (mailto:/tel:/data:, or an unresolvable ref) is
                // not a same-origin page → external, never silently internal.
                _ => false,
            };
            let link = Link {
                href: abs,
                text: tag.inner_text(parser).trim().to_string(),
                title: tag
                    .attributes()
                    .get("title")
                    .flatten()
                    .map(|b| b.as_utf8_str().trim().to_string()),
            };
            if is_internal {
                links.internal.push(link);
            } else {
                links.external.push(link);
            }
        }
    }
    links
}

pub fn extract_media(html: &str, base: &str) -> Media {
    let mut media = Media {
        images: Vec::new(),
        videos: Vec::new(),
        audios: Vec::new(),
    };
    let Ok(dom) = parse(html, ParserOptions::default()) else {
        return media;
    };
    let parser = dom.parser();
    let resolve_base = effective_base(&dom, base);

    if let Some(iter) = dom.query_selector("img") {
        for handle in iter {
            let Some(tag) = handle.get(parser).and_then(|n| n.as_tag()) else {
                continue;
            };
            if let Some(src) = tag.attributes().get("src").flatten() {
                if src.as_utf8_str().trim().is_empty() {
                    continue;
                }
                media.images.push(MediaItem {
                    src: resolve(&resolve_base, &src.as_utf8_str()),
                    alt: tag
                        .attributes()
                        .get("alt")
                        .flatten()
                        .map(|b| b.as_utf8_str().trim().to_string()),
                });
            }
        }
    }
    for (sel, bucket) in [("video", &mut media.videos), ("audio", &mut media.audios)] {
        if let Some(iter) = dom.query_selector(sel) {
            for handle in iter {
                let Some(tag) = handle.get(parser).and_then(|n| n.as_tag()) else {
                    continue;
                };
                // src may be on the element directly or (the common case) on a child
                // <source src=…>; prefer the direct attribute, then the first source.
                let direct = tag
                    .attributes()
                    .get("src")
                    .flatten()
                    .map(|b| b.as_utf8_str().into_owned())
                    .filter(|s| !s.trim().is_empty());
                if let Some(src) = direct.or_else(|| source_child_src(tag, parser)) {
                    bucket.push(MediaItem {
                        src: resolve(&resolve_base, &src),
                        alt: None,
                    });
                }
            }
        }
    }
    media
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_and_classifies_links() {
        let html = r#"<a href="/about">About</a><a href="https://other.test/x">Ext</a>"#;
        let links = extract_links(html, "https://site.test/page");
        assert_eq!(links.internal.len(), 1);
        assert_eq!(links.internal[0].href, "https://site.test/about");
        assert_eq!(links.internal[0].text, "About");
        assert_eq!(links.external.len(), 1);
        assert_eq!(links.external[0].href, "https://other.test/x");
    }

    #[test]
    fn extracts_images_with_alt() {
        let html = r#"<img src="/img/a.png" alt="logo"><img src="b.jpg">"#;
        let media = extract_media(html, "https://site.test/dir/page");
        assert_eq!(media.images[0].src, "https://site.test/img/a.png");
        assert_eq!(media.images[0].alt.as_deref(), Some("logo"));
        assert_eq!(media.images[1].src, "https://site.test/dir/b.jpg");
        assert!(media.images[1].alt.is_none());
    }

    #[test]
    fn resolves_against_base_href_and_classifies_by_fetch_host() {
        // <base href> off-site: relative URLs resolve against it (per spec) and,
        // being off the fetch host, are external — not internal.
        let html = r#"<head><base href="https://cdn.other.test/app/"></head>
            <body><a href="x">rel</a><a href="/root">root</a></body>"#;
        let links = extract_links(html, "https://site.test/docs/page");
        assert!(links.internal.is_empty());
        let ext: Vec<&str> = links.external.iter().map(|l| l.href.as_str()).collect();
        assert!(ext.contains(&"https://cdn.other.test/app/x"));
        assert!(ext.contains(&"https://cdn.other.test/root"));
    }

    #[test]
    fn mailto_is_external_not_internal() {
        let html = r#"<a href="mailto:a@b.com">mail</a><a href="/x">x</a>"#;
        let links = extract_links(html, "https://site.test/");
        assert_eq!(links.internal.len(), 1);
        assert_eq!(links.internal[0].href, "https://site.test/x");
        assert!(links.external.iter().any(|l| l.href == "mailto:a@b.com"));
    }

    #[test]
    fn video_audio_source_children_extracted() {
        let html = r#"<video><source src="/clip.mp4"></video>
            <audio><source src="/a.mp3"></audio>
            <video src="/direct.webm"></video>"#;
        let media = extract_media(html, "https://site.test/");
        let vids: Vec<&str> = media.videos.iter().map(|m| m.src.as_str()).collect();
        assert!(vids.contains(&"https://site.test/clip.mp4")); // via <source> child
        assert!(vids.contains(&"https://site.test/direct.webm")); // direct src still works
        assert_eq!(media.audios[0].src, "https://site.test/a.mp3"); // audio <source>
    }
}
