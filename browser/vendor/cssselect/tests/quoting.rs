//! XPath string-literal quoting, unicode survival, and unicode escapes.

use cssselect::GenericTranslator;

/// Translate with the default prefix.
fn xpath(css: &str) -> String {
    GenericTranslator::new().css_to_xpath(css).unwrap()
}

#[test]
fn unicode_class_survives() {
    let css = ".a\u{c1}b";
    let result = xpath(css);
    assert!(result.contains("a\u{c1}b"));
    // The ASCII transcription uses XML character references for non-ASCII.
    let ascii = ascii_xmlcharref(&result);
    assert_eq!(
        ascii,
        "descendant-or-self::*[@class and contains(\
concat(' ', normalize-space(@class), ' '), ' a&#193;b ')]"
    );
}

/// Replace every non-ASCII character with its `&#N;` XML character reference.
fn ascii_xmlcharref(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if c.is_ascii() {
            out.push(c);
        } else {
            out.push_str(&format!("&#{};", c as u32));
        }
    }
    out
}

#[test]
fn quote_selection() {
    assert_eq!(
        xpath("*[aval=\"'\"]"),
        "descendant-or-self::*[@aval = \"'\"]"
    );
    assert_eq!(
        xpath("*[aval=\"'''\"]"),
        "descendant-or-self::*[@aval = \"'''\"]"
    );
    assert_eq!(xpath("*[aval='\"']"), "descendant-or-self::*[@aval = '\"']");
    assert_eq!(
        xpath("*[aval='\"\"\"']"),
        "descendant-or-self::*[@aval = '\"\"\"']"
    );
    assert_eq!(
        xpath(":scope > div[dataimg=\"<testmessage>\"]"),
        "descendant-or-self::*[1]/div[@dataimg = '<testmessage>']"
    );
}

#[test]
fn unicode_escapes() {
    // \22 is a double quote, \20 is a space.
    assert_eq!(
        xpath(r#"*[aval="\'\22\'"]"#),
        "descendant-or-self::*[@aval = concat(\"'\",'\"',\"'\")]"
    );
    assert_eq!(
        xpath(r#"*[aval="\'\22 2\'"]"#),
        "descendant-or-self::*[@aval = concat(\"'\",'\"2',\"'\")]"
    );
    assert_eq!(
        xpath(r#"*[aval="\'\20  \'"]"#),
        "descendant-or-self::*[@aval = \"'  '\"]"
    );
    assert_eq!(
        xpath("*[aval=\"'\\20\r\n '\"]"),
        "descendant-or-self::*[@aval = \"'  '\"]"
    );
}

/// Translate with an empty prefix.
fn xpath_bare(css: &str) -> String {
    GenericTranslator::new()
        .css_to_xpath_with_prefix(css, "")
        .unwrap()
}

#[test]
fn both_quotes_force_concat() {
    // A value with both quote kinds builds a concat() through the real
    // attribute-equals path, not only the helper.
    assert_eq!(
        xpath_bare("*[a=\"it's a \\\"q\\\"\"]"),
        "*[@a = concat('it',\"'\",'s a \"q\"')]"
    );
    // The same split feeds :contains().
    assert_eq!(xpath_bare("*:contains(\"a'b\")"), "*[contains(., \"a'b\")]");
}
