use xmloxide::html::{parse_html_with_options, HtmlParseOptions};
use xmloxide::serial::html::serialize_html_subtree;
use xmloxide::tree::NodeKind;

fn parse(input: &str) -> xmloxide::Document {
    parse_html_with_options(
        input,
        &HtmlParseOptions::default().recover(true).no_blanks(true),
    )
    .unwrap()
}

fn first(doc: &xmloxide::Document, name: &str) -> xmloxide::NodeId {
    std::iter::once(doc.root())
        .chain(doc.descendants(doc.root()))
        .find(|id| doc.node_name(*id) == Some(name))
        .unwrap()
}

#[test]
fn html_recovery_and_subtree_serialization_match_the_oracle() {
    let cases = [
        (
            "<table><td>A<td>B<div>C",
            "table",
            "<table><td>A</td><td>B<div>C</div></td></table>",
        ),
        (
            "<p><b>one<i>two</b>three</i>tail",
            "body",
            "<body><p><b>one<i>two</i></b>threetail</p></body>",
        ),
        (
            "<svg viewBox='0 0 1 1'><foreignObject><DIV xlink:href='x'>T</DIV></foreignObject></svg>",
            "svg",
            "<svg viewbox=\"0 0 1 1\"><foreignobject><div xlink:href=\"x\">T</div></foreignobject></svg>",
        ),
        (
            "<template><table><td>T</template><p>P",
            "template",
            "<template><table><td>T<p>P</p></td></table></template>",
        ),
    ];
    for (input, tag, expected) in cases {
        let doc = parse(input);
        assert_eq!(serialize_html_subtree(&doc, first(&doc, tag)), expected);
    }
}

#[test]
fn parser_preserves_attribute_order_and_first_duplicate() {
    let doc = parse("<input z=1 disabled a='' z=2 checked=checked>");
    let input = first(&doc, "input");
    let attrs: Vec<_> = doc
        .attributes(input)
        .iter()
        .map(|attr| (attr.name.as_str(), attr.value.as_str()))
        .collect();
    assert_eq!(
        attrs,
        [
            ("z", "1"),
            ("disabled", "disabled"),
            ("a", ""),
            ("checked", "checked")
        ]
    );
    assert_eq!(
        serialize_html_subtree(&doc, input),
        "<input z=\"1\" disabled a=\"\" checked>"
    );
}

#[test]
fn parser_can_drop_comments_and_cdata_like_scrapling() {
    let mut doc = parse("<p>a<!--gone-->b<![CDATA[c]]>d</p>");
    let removed: Vec<_> = doc
        .descendants(doc.root())
        .filter(|id| {
            matches!(
                doc.node(*id).kind,
                NodeKind::Comment { .. } | NodeKind::CData { .. }
            )
        })
        .collect();
    for id in removed {
        doc.remove_node(id);
    }
    let p = first(&doc, "p");
    assert_eq!(doc.text_content(p), "abd");
    assert_eq!(serialize_html_subtree(&doc, p), "<p>abd</p>");
}
