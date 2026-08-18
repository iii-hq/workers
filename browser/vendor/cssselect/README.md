# cssselect

Translate CSS3 selectors into XPath 1.0 expression strings.

This crate parses a CSS group of selectors and emits XPath 1.0 strings. It does
no DOM matching and no I/O. The whole library is a pure string to string
transformation. It needs `alloc` but not `std`, so it builds for `no_std` and
`wasm32` targets with zero runtime dependencies.

## Installation

```toml
[dependencies]
cssselect = "0.1"
```

## Usage

```rust
use cssselect::GenericTranslator;

let xpath = GenericTranslator::new()
    .css_to_xpath("a#bar")
    .unwrap();
assert_eq!(xpath, "descendant-or-self::a[@id = 'bar']");
```

`GenericTranslator` targets generic XML. `HtmlTranslator` targets (X)HTML with
case-insensitive names and HTML-aware results for `:checked`, `:disabled`,
`:enabled`, `:link`, and `:lang()`.

Call `parse` directly when you need the parsed tree, the canonical CSS form, or
the specificity:

```rust
use cssselect::parse;

let selector = &parse(":is(.foo, #bar)").unwrap()[0];
assert_eq!(selector.canonical(), ":is(.foo, #bar)");
assert_eq!(selector.specificity(), (1, 0, 0));
```

## Supported selectors

Type, universal, namespace, class, id, and attribute selectors with the `=`,
`~=`, `|=`, `^=`, `$=`, `*=`, and `!=` operators. Structural pseudo-classes
including the `:nth-*` family, `:not()`, `:is()`, `:matches()`, `:where()`,
`:has()`, and `:scope`. Pseudo-elements with one or two colons. The four
combinators: descendant, `>`, `+`, and `~`.

## Tests

`cargo test` runs the string-parity suite with no dependencies. The optional
selection tier runs generated XPath against a real engine:

```sh
cargo test --features xpath-engine-tests
```

## License

Licensed under the [BSD 3-Clause license](LICENSE).
