//! (X)HTML translator.
//!
//! This translator reuses the generic engine with HTML-specific settings. It
//! folds element and attribute names to lower case unless built for XHTML, uses
//! the `lang` attribute for `:lang()`, and gives HTML-aware results for
//! `:checked`, `:disabled`, `:enabled`, and `:link`.

use alloc::string::String;

use crate::error::SelectorError;
use crate::parser::Selector;
use crate::xpath::{css_to_xpath, selector_to_xpath, Config, PseudoElements};

/// Translator that targets (X)HTML.
///
/// Folds element and attribute names to lower case unless built for XHTML.
#[derive(Debug, Clone)]
pub struct HtmlTranslator {
    config: Config,
    xhtml: bool,
}

impl Default for HtmlTranslator {
    fn default() -> Self {
        HtmlTranslator::new()
    }
}

impl HtmlTranslator {
    /// Create an HTML translator. Element and attribute names fold to lower case.
    pub fn new() -> Self {
        HtmlTranslator::with_xhtml(false)
    }

    /// Create a translator with the XHTML flag set as given.
    ///
    /// With `xhtml = true`, names stay case sensitive.
    pub fn with_xhtml(xhtml: bool) -> Self {
        HtmlTranslator {
            config: Config::html(xhtml),
            xhtml,
        }
    }

    /// Whether this translator treats input as XHTML.
    pub fn is_xhtml(&self) -> bool {
        self.xhtml
    }

    /// Translate a CSS group of selectors to an XPath string.
    ///
    /// The default prefix scopes selectors to the context node's subtree.
    pub fn css_to_xpath(&self, css: &str) -> Result<String, SelectorError> {
        css_to_xpath(&self.config, css, "descendant-or-self::")
    }

    /// Translate a CSS group of selectors using an explicit prefix.
    pub fn css_to_xpath_with_prefix(
        &self,
        css: &str,
        prefix: &str,
    ) -> Result<String, SelectorError> {
        css_to_xpath(&self.config, css, prefix)
    }

    /// Translate a single parsed selector to an XPath string.
    ///
    /// The pseudo-element is ignored.
    pub fn selector_to_xpath(&self, selector: &Selector) -> Result<String, SelectorError> {
        selector_to_xpath(
            &self.config,
            selector,
            "descendant-or-self::",
            PseudoElements::Ignore,
        )
    }

    /// Translate a single parsed selector with an explicit prefix and
    /// pseudo-element handling.
    pub fn selector_to_xpath_with(
        &self,
        selector: &Selector,
        prefix: &str,
        pseudo_elements: PseudoElements,
    ) -> Result<String, SelectorError> {
        selector_to_xpath(&self.config, selector, prefix, pseudo_elements)
    }
}
