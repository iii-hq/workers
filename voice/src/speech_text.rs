//! Prepare speech text without interpreting formatting as spoken punctuation.
//! A Markdown parser, rather than deleting every `*`/`_`, preserves literal
//! symbols, escaped characters and inline code. Nothing is rendered or fetched.

use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
use schemars::JsonSchema;
use serde::Deserialize;

/// Input syntax, independent of the voice/language/backend selected for TTS.
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TextFormat {
    /// Read the text content, omitting Markdown markup and fenced code blocks.
    #[default]
    Markdown,
    /// Already-rendered selections or text whose symbols should remain literal.
    Plain,
}

/// Normalize before applying the speech limit, so formatting and hidden link
/// destinations cannot consume the character budget or leave half a delimiter.
pub fn prepare(text: &str, format: TextFormat, max_chars: usize) -> Result<String, String> {
    match format {
        TextFormat::Plain => crate::tts::clip_text(text, max_chars),
        TextFormat::Markdown => crate::tts::clip_text(&markdown_text(text), max_chars),
    }
}

fn boundary(text: &mut String) {
    if !text.is_empty() && !text.ends_with(char::is_whitespace) {
        text.push(' ');
    }
}

/// Extract visible prose. Keep link labels, image alt text and inline-code
/// content; omit destinations, block code, raw HTML blocks and footnote bodies.
fn markdown_text(text: &str) -> String {
    let options = Options::ENABLE_TABLES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_FOOTNOTES;
    let mut output = String::new();
    let mut suppressed = 0usize;
    for event in Parser::new_ext(text, options) {
        if suppressed > 0 {
            match event {
                Event::Start(_) => suppressed += 1,
                Event::End(_) => suppressed -= 1,
                _ => {}
            }
            continue;
        }
        match event {
            Event::Start(Tag::CodeBlock(_) | Tag::HtmlBlock | Tag::FootnoteDefinition(_)) => {
                boundary(&mut output);
                suppressed = 1;
            }
            Event::Text(value) | Event::Code(value) => output.push_str(&value),
            Event::SoftBreak | Event::HardBreak | Event::Rule => boundary(&mut output),
            Event::End(
                TagEnd::Paragraph
                | TagEnd::Heading(_)
                | TagEnd::BlockQuote(_)
                | TagEnd::Item
                | TagEnd::List(_)
                | TagEnd::TableCell
                | TagEnd::TableHead
                | TagEnd::TableRow
                | TagEnd::Table,
            ) => boundary(&mut output),
            Event::InlineHtml(tag)
                if matches!(
                    tag.to_ascii_lowercase().as_str(),
                    "<br>" | "<br/>" | "<br />" | "<hr>" | "<hr/>" | "<hr />"
                ) =>
            {
                boundary(&mut output)
            }
            _ => {}
        }
    }
    output.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spoken(text: &str) -> String {
        prepare(text, TextFormat::Markdown, 4000).unwrap()
    }

    #[test]
    fn emphasis_is_read_without_asterisks_or_underscores() {
        assert_eq!(
            spoken("Use **Piper** para *ler* e __ouvir__ com _clareza_."),
            "Use Piper para ler e ouvir com clareza."
        );
        assert_eq!(
            spoken("***Muito importante***: **voz com *ênfase* dentro**."),
            "Muito importante: voz com ênfase dentro."
        );
    }

    #[test]
    fn headings_lists_quotes_and_checkboxes_keep_words_not_markers() {
        assert_eq!(spoken("# Resumo\n\n> **Pronto.**\n\n- [x] Voz instalada\n- [ ] Fazer teste\n\n---\n\n1. Primeiro\n2. Segundo"),
            "Resumo Pronto. Voz instalada Fazer teste Primeiro Segundo");
        assert_eq!(
            spoken("Título\n======\n\n~~antigo~~ novo"),
            "Título antigo novo"
        );
    }

    #[test]
    fn links_keep_labels_and_do_not_read_hidden_addresses() {
        assert_eq!(
            spoken("Veja [o **manual**](https://example.test/a_(b)?secret=abc)."),
            "Veja o manual."
        );
        assert_eq!(
            spoken("Leia [aqui][ref].\n\n[ref]: https://example.test/long-url"),
            "Leia aqui."
        );
        assert_eq!(
            spoken("![Diagrama da voz](https://example.test/chart.png)"),
            "Diagrama da voz"
        );
    }

    #[test]
    fn table_cells_and_paragraphs_remain_separated() {
        assert_eq!(
            spoken("| Modelo | Idioma |\n| --- | --- |\n| **Faber** | Português |\n\nFim."),
            "Modelo Idioma Faber Português Fim."
        );
    }

    #[test]
    fn code_blocks_are_omitted_but_inline_code_is_read() {
        assert_eq!(
            spoken(
                "Execute `piper --help`.\n\n```python\nprint('**não ler**')\n```\n\nDepois teste."
            ),
            "Execute piper --help. Depois teste."
        );
        assert_eq!(
            spoken("Antes.\n\n~~~rust\nfn main() {}\n~~~\n\nDepois."),
            "Antes. Depois."
        );
        assert_eq!(spoken("Texto.\n\n```\nbloco incompleto"), "Texto.");
        assert!(prepare("```\napenas código\n```", TextFormat::Markdown, 100).is_err());
    }

    #[test]
    fn literal_symbols_are_not_blindly_deleted() {
        assert_eq!(
            spoken("2 * 3 = 6; snake_case; R$ 5,00; C++; 50%."),
            "2 * 3 = 6; snake_case; R$ 5,00; C++; 50%."
        );
        assert_eq!(
            spoken(r"Um \* literal e `**literal**`."),
            "Um * literal e **literal**."
        );
        assert_eq!(spoken("A &amp; B &lt; C."), "A & B < C.");
    }

    #[test]
    fn plain_input_preserves_rendered_selection_and_markup_literally() {
        assert_eq!(
            prepare(" **literal**\n2 * 3 ", TextFormat::Plain, 100).unwrap(),
            "**literal**\n2 * 3"
        );
    }

    #[test]
    fn limit_applies_after_markdown_conversion_and_keeps_unicode() {
        let input = "**Olá** [mundo](https://example.test/long)";
        assert_eq!(
            prepare(input, TextFormat::Markdown, 9).unwrap(),
            "Olá mundo"
        );
        assert!(prepare(input, TextFormat::Markdown, 5).is_err());
        assert!(prepare("---", TextFormat::Markdown, 100).is_err());
    }

    #[test]
    fn raw_html_is_not_spoken_or_executed() {
        assert_eq!(
            spoken("Texto <b>importante</b><br>Fim."),
            "Texto importante Fim."
        );
        assert_eq!(
            spoken("Antes.\n\n<script>alert('nunca executar')</script>\n\nDepois."),
            "Antes. Depois."
        );
    }
}
