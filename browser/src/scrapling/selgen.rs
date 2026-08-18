//! scrapling's generated-selector algorithm (core/mixins.py), ported verbatim.

use crate::scrapling::dom::{self, ElementRef};

fn general_selection(el: ElementRef, css: bool, full_path: bool) -> String {
    let mut parts: Vec<String> = Vec::new();
    let mut target = el;
    while let Some(parent) = dom::parent_element(target) {
        if let Some(id) = target.attr("id").filter(|s| !s.is_empty()) {
            if css {
                parts.push(format!("#{id}"));
            } else if full_path {
                parts.push(format!("*[@id='{id}']"));
            } else {
                parts.push(format!("[@id='{id}']"));
            }
            if !full_path {
                parts.reverse();
                return if css {
                    parts.join(" > ")
                } else {
                    format!("//*{}", parts.join("/"))
                };
            }
        } else {
            let tag = target.name();
            let mut index = 0usize;
            for sibling in dom::element_children(parent) {
                if sibling.name() == tag {
                    index += 1;
                }
                if sibling.id() == target.id() {
                    break;
                }
            }
            let mut part = tag.to_string();
            if index > 1 {
                if css {
                    part.push_str(&format!(":nth-of-type({index})"));
                } else {
                    part.push_str(&format!("[{index}]"));
                }
            }
            parts.push(part);
        }
        target = parent;
        if target.name() == "html" {
            break;
        }
    }
    parts.reverse();
    if css {
        parts.join(" > ")
    } else {
        format!("//{}", parts.join("/"))
    }
}

pub fn css_selector(el: ElementRef) -> String {
    general_selection(el, true, false)
}
pub fn full_css_selector(el: ElementRef) -> String {
    general_selection(el, true, true)
}
pub fn xpath_selector(el: ElementRef) -> String {
    general_selection(el, false, false)
}
pub fn full_xpath_selector(el: ElementRef) -> String {
    general_selection(el, false, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn matches<'a>(d: &'a dom::Doc, selector: &str) -> Vec<ElementRef<'a>> {
        crate::scrapling::query::css_query(d, None, selector)
            .unwrap()
            .into_iter()
            .filter_map(|result| match result {
                crate::scrapling::query::QueryResult::Element(element) => Some(element),
                _ => None,
            })
            .collect()
    }

    fn first<'a>(d: &'a dom::Doc, selector: &str) -> ElementRef<'a> {
        matches(d, selector)[0]
    }

    const HTML: &str = r#"<html><body><h1 class="t">Hello</h1><ul><li><a href="/a">Apple</a></li><li><a href="/b">Banana</a></li></ul></body></html>"#;

    #[test]
    fn plain_chain_stops_below_html() {
        let d = dom::parse(HTML);
        assert_eq!(css_selector(first(&d, "a")), "body > ul > li > a");
        assert_eq!(xpath_selector(first(&d, "a")), "//body/ul/li/a");
    }

    #[test]
    fn nth_of_type_only_when_index_gt_1() {
        let d = dom::parse(HTML);
        let second_li = matches(&d, "li")[1];
        assert_eq!(css_selector(second_li), "body > ul > li:nth-of-type(2)");
        assert_eq!(xpath_selector(second_li), "//body/ul/li[2]");
    }

    #[test]
    fn id_short_circuits_short_variants() {
        let d = dom::parse(r#"<html><body><div id="main"><p>x</p><p>y</p></div></body></html>"#);
        let p2 = matches(&d, "p")[1];
        assert_eq!(css_selector(p2), "#main > p:nth-of-type(2)");
        assert_eq!(xpath_selector(p2), "//*[@id='main']/p[2]");
        // full variants do NOT short-circuit:
        assert_eq!(full_css_selector(p2), "body > #main > p:nth-of-type(2)");
        assert_eq!(full_xpath_selector(p2), "//body/*[@id='main']/p[2]");
    }

    #[test]
    fn html_element_yields_empty() {
        let d = dom::parse(HTML);
        assert_eq!(css_selector(d.root()), "");
    }

    #[test]
    fn empty_id_falls_through_to_tag_addressing() {
        // Python's `if target.attrib.get("id"):` is a truthiness check, so
        // `id=""` must NOT take the id branch (verified against scrapling
        // 0.4.9: same output for short and full variants).
        let d = dom::parse(r#"<html><body><div id=""><p>x</p><p>y</p></div></body></html>"#);
        let p2 = matches(&d, "p")[1];
        assert_eq!(css_selector(p2), "body > div > p:nth-of-type(2)");
        assert_eq!(xpath_selector(p2), "//body/div/p[2]");
        assert_eq!(full_css_selector(p2), "body > div > p:nth-of-type(2)");
        assert_eq!(full_xpath_selector(p2), "//body/div/p[2]");
    }

    #[test]
    fn html_element_xpath_variants_yield_double_slash() {
        // css stays "" (no guard needed), but xpath's unconditional
        // "//" + parts.join("/") naturally yields "//" for an empty parts
        // list (verified against scrapling 0.4.9).
        let d = dom::parse(HTML);
        assert_eq!(css_selector(d.root()), "");
        assert_eq!(xpath_selector(d.root()), "//");
        assert_eq!(full_css_selector(d.root()), "");
        assert_eq!(full_xpath_selector(d.root()), "//");
    }
}
