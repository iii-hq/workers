//! CSS selector AST types.

/// A group of selectors separated by commas: `div, p.intro`
#[derive(Debug, Clone)]
pub struct SelectorGroup {
    /// Individual selectors in the group.
    pub selectors: Vec<Selector>,
}

/// A single selector: a chain of compound selectors joined by combinators.
///
/// For example, `div > p.intro` is a chain of two compounds:
/// `div` (followed by child combinator) and `p.intro`.
#[derive(Debug, Clone)]
pub struct Selector {
    /// The chain of compound selectors and combinators.
    pub compounds: Vec<CompoundEntry>,
}

/// An entry in the selector chain: a compound selector with its leading combinator.
#[derive(Debug, Clone)]
pub struct CompoundEntry {
    /// How this compound relates to the previous one.
    /// The first entry in a chain uses `Combinator::None`.
    pub combinator: Combinator,
    /// The compound selector itself.
    pub compound: CompoundSelector,
}

/// A compound selector: a set of simple selectors that all apply to the same element.
///
/// For example, `p.intro#first[lang]` has tag=`p`, classes=\[`intro`\],
/// id=`first`, and attrs=\[`lang`\].
#[derive(Debug, Clone, Default)]
pub struct CompoundSelector {
    /// Tag name matcher (e.g., `div`). `None` means any tag (implicit `*`).
    pub tag: Option<String>,
    /// ID matcher (e.g., `#main`).
    pub id: Option<String>,
    /// Class matchers (e.g., `.intro`).
    pub classes: Vec<String>,
    /// Attribute matchers (e.g., `[href^="https"]`).
    pub attrs: Vec<AttrSelector>,
    /// Pseudo-class matchers (e.g., `:first-child`).
    pub pseudos: Vec<PseudoClass>,
}

/// Combinator between compound selectors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Combinator {
    /// No combinator (first in chain).
    None,
    /// Descendant combinator (whitespace): `div p`
    Descendant,
    /// Child combinator: `div > p`
    Child,
    /// Adjacent sibling combinator: `div + p`
    NextSibling,
    /// General sibling combinator: `div ~ p`
    SubsequentSibling,
}

/// An attribute selector: `[attr]`, `[attr=value]`, `[attr^=value]`, etc.
#[derive(Debug, Clone)]
pub struct AttrSelector {
    /// Attribute name.
    pub name: String,
    /// Match operator and value. `None` means just `[attr]` (existence check).
    pub matcher: Option<AttrMatcher>,
}

/// Attribute value matching operator and value.
#[derive(Debug, Clone)]
pub struct AttrMatcher {
    /// The match operator.
    pub op: AttrOp,
    /// The value to match against.
    pub value: String,
    /// Case-insensitive flag (`i` modifier).
    pub case_insensitive: bool,
}

/// Attribute match operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttrOp {
    /// `=` — exact match
    Exact,
    /// `~=` — whitespace-separated word match
    Word,
    /// `|=` — exact or prefix followed by `-`
    DashPrefix,
    /// `^=` — starts with
    Prefix,
    /// `$=` — ends with
    Suffix,
    /// `*=` — contains substring
    Substring,
}

/// Pseudo-class selectors.
#[derive(Debug, Clone)]
pub enum PseudoClass {
    /// `:first-child`
    FirstChild,
    /// `:last-child`
    LastChild,
    /// `:only-child`
    OnlyChild,
    /// `:empty`
    Empty,
    /// `:not(selector)`
    Not(Box<CompoundSelector>),
    /// `:nth-child(An+B)`
    NthChild(NthExpr),
    /// `:nth-last-child(An+B)`
    NthLastChild(NthExpr),
}

/// An `An+B` expression for `:nth-child()` and similar.
#[derive(Debug, Clone, Copy)]
pub struct NthExpr {
    /// The `A` coefficient (0 for just `B`).
    pub a: i32,
    /// The `B` offset.
    pub b: i32,
}

impl NthExpr {
    /// Returns true if the 1-based position `pos` matches this `An+B` expression.
    pub fn matches(&self, pos: i32) -> bool {
        if self.a == 0 {
            return pos == self.b;
        }
        let diff = pos - self.b;
        // diff must be divisible by a and have the same sign
        diff % self.a == 0 && diff / self.a >= 0
    }
}
