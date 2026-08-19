//! HTML translator output: case folding, the `lang` attribute, and the
//! HTML-specific pseudo-class blocks.

use cssselect::{GenericTranslator, HtmlTranslator};

/// Translate with the HTML translator and an empty prefix.
fn html(css: &str) -> String {
    HtmlTranslator::new()
        .css_to_xpath_with_prefix(css, "")
        .unwrap()
}

/// Translate with the XHTML translator and an empty prefix.
fn xhtml(css: &str) -> String {
    HtmlTranslator::with_xhtml(true)
        .css_to_xpath_with_prefix(css, "")
        .unwrap()
}

#[test]
fn element_names_fold_to_lower_case() {
    assert_eq!(html("DIV"), "div");
    assert_eq!(html("A[NAme]"), "a[@name]");
    // XHTML keeps names as written.
    assert_eq!(xhtml("DIV"), "DIV");
    assert_eq!(xhtml("A[NAme]"), "A[@NAme]");
}

#[test]
fn link_pseudo() {
    assert_eq!(
        html(":link"),
        "*[@href and (name(.) = 'a' or name(.) = 'link' or name(.) = 'area')]"
    );
    // The generic translator never matches :link.
    assert_eq!(
        GenericTranslator::new()
            .css_to_xpath_with_prefix(":link", "")
            .unwrap(),
        "*[0]"
    );
}

#[test]
fn visited_pseudo_never_matches() {
    assert_eq!(html(":visited"), "*[0]");
}

#[test]
fn checked_pseudo() {
    assert_eq!(
        html(":checked"),
        "*[(@selected and name(.) = 'option') or (@checked and (name(.) = 'input' or name(.) = 'command')and (@type = 'checkbox' or @type = 'radio'))]"
    );
}

#[test]
fn lang_function_uses_lang_attribute() {
    assert_eq!(
        html(":lang(en)"),
        "*[ancestor-or-self::*[@lang][1][starts-with(concat(translate(@lang, 'ABCDEFGHIJKLMNOPQRSTUVWXYZ', 'abcdefghijklmnopqrstuvwxyz'), '-'), 'en-')]]"
    );
}

#[test]
fn disabled_pseudo_block() {
    let expected = "*[\n        (\n            @disabled and\n            (\n                (name(.) = 'input' and @type != 'hidden') or\n                name(.) = 'button' or\n                name(.) = 'select' or\n                name(.) = 'textarea' or\n                name(.) = 'command' or\n                name(.) = 'fieldset' or\n                name(.) = 'optgroup' or\n                name(.) = 'option'\n            )\n        ) or (\n            (\n                (name(.) = 'input' and @type != 'hidden') or\n                name(.) = 'button' or\n                name(.) = 'select' or\n                name(.) = 'textarea'\n            )\n            and ancestor::fieldset[@disabled]\n        )\n        ]";
    assert_eq!(html(":disabled"), expected);
}

#[test]
fn enabled_pseudo_block() {
    let expected = "*[\n        (\n            @href and (\n                name(.) = 'a' or\n                name(.) = 'link' or\n                name(.) = 'area'\n            )\n        ) or (\n            (\n                name(.) = 'command' or\n                name(.) = 'fieldset' or\n                name(.) = 'optgroup'\n            )\n            and not(@disabled)\n        ) or (\n            (\n                (name(.) = 'input' and @type != 'hidden') or\n                name(.) = 'button' or\n                name(.) = 'select' or\n                name(.) = 'textarea' or\n                name(.) = 'keygen'\n            )\n            and not (@disabled or ancestor::fieldset[@disabled])\n        ) or (\n            name(.) = 'option' and not(\n                @disabled or ancestor::optgroup[@disabled]\n            )\n        )\n        ]";
    assert_eq!(html(":enabled"), expected);
}
