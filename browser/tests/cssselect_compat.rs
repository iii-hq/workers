use cssselect::HtmlTranslator;

#[test]
fn scrapling_pseudo_elements_translate_to_xpath() {
    let translator = HtmlTranslator::new();
    for (css, xpath) in [
        ("a::text", "descendant-or-self::a/text()"),
        (
            "a ::text",
            "descendant-or-self::a/descendant-or-self::text()",
        ),
        ("::text", "descendant-or-self::text()"),
        ("a::attr(href)", "descendant-or-self::a/@href"),
        (
            "a ::attr(href)",
            "descendant-or-self::a/descendant-or-self::*/@href",
        ),
        (
            "h1 + p::text",
            "descendant-or-self::h1/following-sibling::*[(self::p) and (position() = 1)]/text()",
        ),
        (
            "h1 ~ p::attr(data-x)",
            "descendant-or-self::h1/following-sibling::p/@data-x",
        ),
    ] {
        assert_eq!(translator.css_to_xpath(css).unwrap(), xpath, "{css}");
    }
}
