//! Translate CSS3 selectors into XPath 1.0 expression strings.
//!
//! This crate parses a CSS group of selectors into an abstract syntax tree and
//! translates each selector into an XPath 1.0 string. It does no DOM matching
//! and no I/O. The whole library is a pure string to string transformation.
//!
//! Two translators are available. [`GenericTranslator`] targets generic XML and
//! is fully case sensitive. [`HtmlTranslator`] targets (X)HTML, folds element
//! and attribute names to lower case unless built with `xhtml = true`, and gives
//! HTML aware results for `:checked`, `:disabled`, `:enabled`, `:link`, and
//! `:lang()`.
//!
//! # Example
//!
//! ```
//! use cssselect::GenericTranslator;
//!
//! let xpath = GenericTranslator::new()
//!     .css_to_xpath("div.foo > a#bar")
//!     .unwrap();
//! assert_eq!(
//!     xpath,
//!     "descendant-or-self::div[@class and contains(\
//! concat(' ', normalize-space(@class), ' '), ' foo ')]/a[@id = 'bar']"
//! );
//! ```
//!
//! The crate is `no_std` friendly. It needs `alloc` but not `std`. Build with
//! `--no-default-features` to drop the `std` feature.
#![no_std]
#![forbid(unsafe_code)]
#![warn(missing_docs)]

extern crate alloc;

#[cfg(feature = "std")]
extern crate std;

pub mod error;
pub mod html;
pub mod parser;
pub mod tokenizer;
mod util;
pub mod xpath;

pub use error::SelectorError;
pub use html::HtmlTranslator;
pub use parser::{
    parse, parse_series, FunctionalPseudoElement, PseudoElement, Selector, Specificity, Tree,
};
pub use tokenizer::{tokenize, Token, TokenType};
pub use xpath::{xpath_literal, Config, GenericTranslator, PseudoElements, XpathExpr};
