//! WHATWG HTML5 tree construction algorithm.
//!
//! This module implements the tree construction stage of the HTML parsing
//! algorithm as defined in the WHATWG HTML Living Standard. It consumes
//! tokens from the [`Tokenizer`] and builds a [`Document`] tree.
//!
//! See <https://html.spec.whatwg.org/multipage/parsing.html#tree-construction>

use crate::error::{ErrorSeverity, ParseDiagnostic, ParseError, SourceLocation};
use crate::html5::tokenizer::{self, State, Token, Tokenizer};
use crate::tree::{Document, NodeId, NodeKind};

// ---------------------------------------------------------------------------
// Insertion mode
// ---------------------------------------------------------------------------

/// All insertion modes from the WHATWG specification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InsertionMode {
    Initial,
    BeforeHtml,
    BeforeHead,
    InHead,
    InHeadNoscript,
    AfterHead,
    InBody,
    Text,
    InTable,
    InTableText,
    InCaption,
    InColumnGroup,
    InTableBody,
    InRow,
    InCell,
    InSelect,
    InSelectInTable,
    InTemplate,
    AfterBody,
    InFrameset,
    AfterFrameset,
    AfterAfterBody,
    AfterAfterFrameset,
}

// ---------------------------------------------------------------------------
// Quirks mode
// ---------------------------------------------------------------------------

/// Document compatibility mode determined by the DOCTYPE.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum QuirksMode {
    NoQuirks,
    Quirks,
    LimitedQuirks,
}

// Full quirks mode public identifier prefixes
const QUIRKS_PREFIXES: &[&str] = &[
    "+//silmaril//dtd html pro v0r11 19970101//",
    "-//as//dtd html 3.0 aswedit extensions//",
    "-//advasoft ltd//dtd html 3.0 aswedit + extensions//",
    "-//ietf//dtd html 2.0 level 1//",
    "-//ietf//dtd html 2.0 level 2//",
    "-//ietf//dtd html 2.0 strict level 1//",
    "-//ietf//dtd html 2.0 strict level 2//",
    "-//ietf//dtd html 2.0 strict//",
    "-//ietf//dtd html 2.0//",
    "-//ietf//dtd html 2.1e//",
    "-//ietf//dtd html 3.0//",
    "-//ietf//dtd html 3.2 final//",
    "-//ietf//dtd html 3.2//",
    "-//ietf//dtd html 3//",
    "-//ietf//dtd html level 0//",
    "-//ietf//dtd html level 1//",
    "-//ietf//dtd html level 2//",
    "-//ietf//dtd html level 3//",
    "-//ietf//dtd html strict level 0//",
    "-//ietf//dtd html strict level 1//",
    "-//ietf//dtd html strict level 2//",
    "-//ietf//dtd html strict level 3//",
    "-//ietf//dtd html strict//",
    "-//ietf//dtd html//",
    "-//metrius//dtd metrius presentational//",
    "-//microsoft//dtd internet explorer 2.0 html strict//",
    "-//microsoft//dtd internet explorer 2.0 html//",
    "-//microsoft//dtd internet explorer 2.0 tables//",
    "-//microsoft//dtd internet explorer 3.0 html strict//",
    "-//microsoft//dtd internet explorer 3.0 html//",
    "-//microsoft//dtd internet explorer 3.0 tables//",
    "-//netscape comm. corp.//dtd html//",
    "-//netscape comm. corp.//dtd strict html//",
    "-//o'reilly and associates//dtd html 2.0//",
    "-//o'reilly and associates//dtd html extended 1.0//",
    "-//o'reilly and associates//dtd html extended relaxed 1.0//",
    "-//sq//dtd html 2.0 hotmetal + extensions//",
    "-//softquad software//dtd hotmetal pro 6.0::19990601::extensions to html 4.0//",
    "-//softquad//dtd hotmetal pro 4.0::19971010::extensions to html 4.0//",
    "-//spyglass//dtd html 2.0 extended//",
    "-//sun microsystems corp.//dtd hotjava html//",
    "-//sun microsystems corp.//dtd hotjava strict html//",
    "-//w3c//dtd html 3 1995-03-24//",
    "-//w3c//dtd html 3.2 draft//",
    "-//w3c//dtd html 3.2 final//",
    "-//w3c//dtd html 3.2//",
    "-//w3c//dtd html 3.2s draft//",
    "-//w3c//dtd html 4.0 frameset//",
    "-//w3c//dtd html 4.0 transitional//",
    "-//w3c//dtd html experimental 19960712//",
    "-//w3c//dtd html experimental 970421//",
    "-//w3c//dtd w3 html//",
    "-//w3o//dtd w3 html 3.0//",
    "-//webtechs//dtd mozilla html 2.0//",
    "-//webtechs//dtd mozilla html//",
];

// Exact matches for quirks
const QUIRKS_EXACT: &[&str] = &[
    "-//w3o//dtd w3 html strict 3.0//en//",
    "-/w3c/dtd html 4.0 transitional/en",
    "html",
];

/// Determine quirks mode from a DOCTYPE token per WHATWG §13.2.6.4.1.
fn determine_quirks_mode(
    name: &str,
    public_id: Option<&str>,
    system_id: Option<&str>,
    force_quirks: bool,
) -> QuirksMode {
    if force_quirks {
        return QuirksMode::Quirks;
    }
    if !name.eq_ignore_ascii_case("html") {
        return QuirksMode::Quirks;
    }
    let pub_id = public_id.unwrap_or("");
    let pub_lower = pub_id.to_ascii_lowercase();

    let sys_id = system_id.unwrap_or("");
    let sys_lower = sys_id.to_ascii_lowercase();

    if sys_lower == "http://www.ibm.com/data/dtd/v11/ibmxhtml1-transitional.dtd" {
        return QuirksMode::Quirks;
    }

    for exact in QUIRKS_EXACT {
        if pub_lower == *exact {
            return QuirksMode::Quirks;
        }
    }

    for prefix in QUIRKS_PREFIXES {
        if pub_lower.starts_with(prefix) {
            return QuirksMode::Quirks;
        }
    }

    // Quirks if these prefixes appear and system identifier is missing
    if system_id.is_none()
        && (pub_lower.starts_with("-//w3c//dtd html 4.01 frameset//")
            || pub_lower.starts_with("-//w3c//dtd html 4.01 transitional//"))
    {
        return QuirksMode::Quirks;
    }

    // Limited quirks mode
    if pub_lower.starts_with("-//w3c//dtd xhtml 1.0 frameset//")
        || pub_lower.starts_with("-//w3c//dtd xhtml 1.0 transitional//")
    {
        return QuirksMode::LimitedQuirks;
    }
    if system_id.is_some()
        && (pub_lower.starts_with("-//w3c//dtd html 4.01 frameset//")
            || pub_lower.starts_with("-//w3c//dtd html 4.01 transitional//"))
    {
        return QuirksMode::LimitedQuirks;
    }

    QuirksMode::NoQuirks
}

// ---------------------------------------------------------------------------
// Namespace
// ---------------------------------------------------------------------------

/// The namespace of an element in the tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Namespace {
    Html,
    Svg,
    MathMl,
}

impl Namespace {
    fn uri(self) -> &'static str {
        match self {
            Self::Html => "http://www.w3.org/1999/xhtml",
            Self::Svg => "http://www.w3.org/2000/svg",
            Self::MathMl => "http://www.w3.org/1998/Math/MathML",
        }
    }
}

// ---------------------------------------------------------------------------
// Active formatting list entry
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
enum FormatEntry {
    Element {
        node_id: NodeId,
        name: String,
        attrs: Vec<tokenizer::Attribute>,
    },
    Marker,
}

// ---------------------------------------------------------------------------
// Tree build error
// ---------------------------------------------------------------------------

/// An error encountered during tree construction.
#[derive(Debug, Clone)]
#[allow(dead_code)]
struct TreeBuildError {
    message: String,
}

// ---------------------------------------------------------------------------
// Helper: element metadata stored alongside NodeId on the open elements stack
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct StackEntry {
    node_id: NodeId,
    name: String,
    ns: Namespace,
    /// True if this is a `MathML` `annotation-xml` element with
    /// `encoding="text/html"` or `encoding="application/xhtml+xml"`.
    is_html_integration: bool,
}

// ---------------------------------------------------------------------------
// Options
// ---------------------------------------------------------------------------

/// Options for HTML5 parsing.
#[derive(Debug, Clone, Default)]
pub struct Html5ParseOptions {
    /// Whether scripting is enabled (affects `<noscript>` handling).
    pub scripting: bool,
    /// If set, parse as a fragment with the given context element tag name.
    ///
    /// Use plain element names for HTML contexts (e.g. `"body"`, `"select"`)
    /// and namespace-prefixed names for foreign contexts (e.g. `"svg svg"`,
    /// `"math mi"`).
    pub fragment_context: Option<String>,
}

/// Result of HTML5 parsing, containing the document tree and any parse errors.
///
/// The WHATWG HTML5 parsing algorithm is designed to handle all input without
/// fatal errors. Parse errors are collected as diagnostics rather than causing
/// failure; the tree is always produced.
///
/// # Examples
///
/// ```
/// use xmloxide::html5::parse_html5_full;
///
/// let result = parse_html5_full("<p>Unclosed paragraph<p>Next");
/// assert!(result.errors.is_empty() || !result.errors.is_empty()); // always succeeds
/// let _doc = result.document; // tree is always available
/// ```
#[derive(Debug)]
pub struct Html5ParseResult {
    /// The constructed document tree.
    pub document: Document,
    /// Parse errors encountered during tokenization and tree construction.
    ///
    /// These are non-fatal diagnostics — the document tree is still complete.
    pub errors: Vec<ParseDiagnostic>,
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Parses an HTML5 string into a [`Document`] with default options.
///
/// # Errors
///
/// Returns [`ParseError`] if parsing fails fatally (extremely rare for HTML5,
/// since the algorithm is designed to handle all input).
///
/// # Examples
///
/// ```
/// use xmloxide::html5::parse_html5;
///
/// let doc = parse_html5("<h1>Hello</h1>").unwrap();
/// let root = doc.root_element().unwrap();
/// assert_eq!(doc.node_name(root), Some("html"));
/// ```
pub fn parse_html5(input: &str) -> Result<Document, ParseError> {
    parse_html5_with_options(input, &Html5ParseOptions::default())
}

/// Parses an HTML5 string into a [`Document`] with the given options.
///
/// # Errors
///
/// Returns [`ParseError`] if parsing fails fatally.
///
/// # Examples
///
/// ```
/// use xmloxide::html5::{parse_html5_with_options, Html5ParseOptions};
///
/// let opts = Html5ParseOptions {
///     scripting: false,
///     fragment_context: Some("body".to_string()),
/// };
/// let doc = parse_html5_with_options("<p>fragment</p>", &opts).unwrap();
/// ```
pub fn parse_html5_with_options(
    input: &str,
    options: &Html5ParseOptions,
) -> Result<Document, ParseError> {
    let result = parse_html5_full_with_options(input, options);
    Ok(result.document)
}

/// Parses an HTML5 string and returns the document tree along with all parse
/// errors.
///
/// Unlike [`parse_html5`], this function always succeeds (no `Result`) and
/// returns collected [`ParseDiagnostic`]s for inspection.
///
/// # Examples
///
/// ```
/// use xmloxide::html5::parse_html5_full;
///
/// let result = parse_html5_full("<p>Hello</p>");
/// println!("{} errors", result.errors.len());
/// ```
pub fn parse_html5_full(input: &str) -> Html5ParseResult {
    parse_html5_full_with_options(input, &Html5ParseOptions::default())
}

/// Parses an HTML5 string with the given options, returning the document tree
/// and all parse errors.
pub fn parse_html5_full_with_options(input: &str, options: &Html5ParseOptions) -> Html5ParseResult {
    let tokenizer = Tokenizer::new(input);
    let mut builder = TreeBuilder::new(tokenizer, options);
    builder.run();

    // Collect tokenizer errors as diagnostics.
    let mut errors: Vec<ParseDiagnostic> = builder
        .tokenizer
        .errors()
        .iter()
        .map(|e| {
            // Compute line/column from byte offset.
            let (line, col) = byte_offset_to_line_col(input, e.span);
            ParseDiagnostic {
                severity: ErrorSeverity::Error,
                message: e.code.to_string(),
                location: SourceLocation {
                    line,
                    column: col,
                    byte_offset: e.span,
                },
            }
        })
        .collect();

    // Append tree construction errors.
    for e in &builder.errors {
        errors.push(ParseDiagnostic {
            severity: ErrorSeverity::Error,
            message: e.message.clone(),
            location: SourceLocation::default(),
        });
    }

    Html5ParseResult {
        document: builder.doc,
        errors,
    }
}

/// Convert a byte offset into 1-based (line, column) pair.
fn byte_offset_to_line_col(input: &str, offset: usize) -> (u32, u32) {
    let bytes = input.as_bytes();
    let end = offset.min(bytes.len());
    let mut line: u32 = 1;
    let mut col: u32 = 1;
    for &b in &bytes[..end] {
        if b == b'\n' {
            line += 1;
            col = 1;
        } else {
            col += 1;
        }
    }
    (line, col)
}

// ---------------------------------------------------------------------------
// Constants: element categories
// ---------------------------------------------------------------------------

fn is_formatting_element(name: &str) -> bool {
    matches!(
        name,
        "a" | "b"
            | "big"
            | "code"
            | "em"
            | "font"
            | "i"
            | "s"
            | "small"
            | "strike"
            | "strong"
            | "tt"
            | "u"
            | "nobr"
    )
}

fn is_special_element_ns(name: &str, ns: Namespace) -> bool {
    match ns {
        Namespace::Html => matches!(
            name,
            "address"
                | "applet"
                | "area"
                | "article"
                | "aside"
                | "base"
                | "basefont"
                | "bgsound"
                | "blockquote"
                | "body"
                | "br"
                | "button"
                | "caption"
                | "center"
                | "col"
                | "colgroup"
                | "dd"
                | "details"
                | "dir"
                | "div"
                | "dl"
                | "dt"
                | "embed"
                | "fieldset"
                | "figcaption"
                | "figure"
                | "footer"
                | "form"
                | "frame"
                | "frameset"
                | "h1"
                | "h2"
                | "h3"
                | "h4"
                | "h5"
                | "h6"
                | "head"
                | "header"
                | "hgroup"
                | "hr"
                | "html"
                | "iframe"
                | "img"
                | "input"
                | "keygen"
                | "li"
                | "link"
                | "listing"
                | "main"
                | "marquee"
                | "menu"
                | "meta"
                | "nav"
                | "noembed"
                | "noframes"
                | "noscript"
                | "object"
                | "ol"
                | "p"
                | "param"
                | "plaintext"
                | "pre"
                | "script"
                | "search"
                | "section"
                | "select"
                | "source"
                | "style"
                | "summary"
                | "table"
                | "tbody"
                | "td"
                | "template"
                | "textarea"
                | "tfoot"
                | "th"
                | "thead"
                | "title"
                | "tr"
                | "track"
                | "ul"
                | "wbr"
                | "xmp"
        ),
        Namespace::MathMl => {
            matches!(name, "mi" | "mo" | "mn" | "ms" | "mtext" | "annotation-xml")
        }
        Namespace::Svg => matches!(name, "foreignObject" | "desc" | "title"),
    }
}

#[allow(dead_code)]
fn is_void_element(name: &str) -> bool {
    matches!(
        name,
        "area"
            | "base"
            | "br"
            | "col"
            | "embed"
            | "hr"
            | "img"
            | "input"
            | "link"
            | "meta"
            | "param"
            | "source"
            | "track"
            | "wbr"
    )
}

fn is_heading(name: &str) -> bool {
    matches!(name, "h1" | "h2" | "h3" | "h4" | "h5" | "h6")
}

// Scope element sets (WHATWG 13.2.4.2)
fn is_scope_element(name: &str, ns: Namespace) -> bool {
    match ns {
        Namespace::Html => matches!(
            name,
            "applet"
                | "caption"
                | "html"
                | "table"
                | "td"
                | "th"
                | "marquee"
                | "object"
                | "template"
        ),
        Namespace::MathMl => matches!(name, "mi" | "mo" | "mn" | "ms" | "mtext" | "annotation-xml"),
        Namespace::Svg => matches!(name, "foreignObject" | "desc" | "title"),
    }
}

fn is_list_item_scope_element(name: &str, ns: Namespace) -> bool {
    is_scope_element(name, ns) || (ns == Namespace::Html && matches!(name, "ol" | "ul"))
}

fn is_button_scope_element(name: &str, ns: Namespace) -> bool {
    is_scope_element(name, ns) || (ns == Namespace::Html && name == "button")
}

fn is_table_scope_element(name: &str, ns: Namespace) -> bool {
    ns == Namespace::Html && matches!(name, "html" | "table" | "template")
}

fn is_select_scope_element(name: &str, ns: Namespace) -> bool {
    // Everything EXCEPT optgroup, option, and elements allowed in the new
    // select content model.
    !(ns == Namespace::Html
        && matches!(
            name,
            "optgroup"
                | "option"
                | "button"
                | "datalist"
                | "div"
                | "selectedcontent"
                | "b"
                | "big"
                | "code"
                | "em"
                | "font"
                | "i"
                | "s"
                | "small"
                | "strike"
                | "strong"
                | "tt"
                | "u"
                | "a"
                | "nobr"
                | "keygen"
                | "menuitem"
                | "hr"
                | "img"
                | "br"
                | "p"
                | "span"
                | "label"
        ))
}

/// Tags that break out of foreign content back to HTML processing.
fn is_foreign_breakout_tag(name: &str) -> bool {
    matches!(
        name,
        "b" | "big"
            | "blockquote"
            | "body"
            | "br"
            | "center"
            | "code"
            | "dd"
            | "details"
            | "dialog"
            | "dir"
            | "div"
            | "dl"
            | "dt"
            | "em"
            | "embed"
            | "h1"
            | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
            | "head"
            | "hr"
            | "i"
            | "img"
            | "li"
            | "listing"
            | "menu"
            | "meta"
            | "nobr"
            | "ol"
            | "p"
            | "pre"
            | "ruby"
            | "s"
            | "small"
            | "span"
            | "strong"
            | "strike"
            | "sub"
            | "sup"
            | "table"
            | "tt"
            | "u"
            | "ul"
            | "var"
    )
}

/// Adjust SVG element names from lowercased parser output to proper camelCase.
fn adjust_svg_tag_name(name: &str) -> String {
    match name {
        "altglyph" => "altGlyph".to_string(),
        "altglyphdef" => "altGlyphDef".to_string(),
        "altglyphitem" => "altGlyphItem".to_string(),
        "animatecolor" => "animateColor".to_string(),
        "animatemotion" => "animateMotion".to_string(),
        "animatetransform" => "animateTransform".to_string(),
        "clippath" => "clipPath".to_string(),
        "feblend" => "feBlend".to_string(),
        "fecolormatrix" => "feColorMatrix".to_string(),
        "fecomponenttransfer" => "feComponentTransfer".to_string(),
        "fecomposite" => "feComposite".to_string(),
        "feconvolvematrix" => "feConvolveMatrix".to_string(),
        "fediffuselighting" => "feDiffuseLighting".to_string(),
        "fedisplacementmap" => "feDisplacementMap".to_string(),
        "fedistantlight" => "feDistantLight".to_string(),
        "fedropshadow" => "feDropShadow".to_string(),
        "feflood" => "feFlood".to_string(),
        "fefunca" => "feFuncA".to_string(),
        "fefuncb" => "feFuncB".to_string(),
        "fefuncg" => "feFuncG".to_string(),
        "fefuncr" => "feFuncR".to_string(),
        "fegaussianblur" => "feGaussianBlur".to_string(),
        "feimage" => "feImage".to_string(),
        "femerge" => "feMerge".to_string(),
        "femergenode" => "feMergeNode".to_string(),
        "femorphology" => "feMorphology".to_string(),
        "feoffset" => "feOffset".to_string(),
        "fepointlight" => "fePointLight".to_string(),
        "fespecularlighting" => "feSpecularLighting".to_string(),
        "fespotlight" => "feSpotLight".to_string(),
        "fetile" => "feTile".to_string(),
        "feturbulence" => "feTurbulence".to_string(),
        "foreignobject" => "foreignObject".to_string(),
        "glyphref" => "glyphRef".to_string(),
        "lineargradient" => "linearGradient".to_string(),
        "radialgradient" => "radialGradient".to_string(),
        "textpath" => "textPath".to_string(),
        _ => name.to_string(),
    }
}

/// Adjust SVG attribute names from lowercased to proper camelCase per WHATWG.
fn adjust_svg_attributes(name: &str) -> &str {
    match name {
        "attributename" => "attributeName",
        "attributetype" => "attributeType",
        "basefrequency" => "baseFrequency",
        "baseprofile" => "baseProfile",
        "calcmode" => "calcMode",
        "clippathunits" => "clipPathUnits",
        "diffuseconstant" => "diffuseConstant",
        "edgemode" => "edgeMode",
        "filterunits" => "filterUnits",
        "glyphref" => "glyphRef",
        "gradienttransform" => "gradientTransform",
        "gradientunits" => "gradientUnits",
        "kernelmatrix" => "kernelMatrix",
        "kernelunitlength" => "kernelUnitLength",
        "keypoints" => "keyPoints",
        "keysplines" => "keySplines",
        "keytimes" => "keyTimes",
        "lengthadjust" => "lengthAdjust",
        "limitingconeangle" => "limitingConeAngle",
        "markerheight" => "markerHeight",
        "markerunits" => "markerUnits",
        "markerwidth" => "markerWidth",
        "maskcontentunits" => "maskContentUnits",
        "maskunits" => "maskUnits",
        "numoctaves" => "numOctaves",
        "pathlength" => "pathLength",
        "patterncontentunits" => "patternContentUnits",
        "patterntransform" => "patternTransform",
        "patternunits" => "patternUnits",
        "pointsatx" => "pointsAtX",
        "pointsaty" => "pointsAtY",
        "pointsatz" => "pointsAtZ",
        "preservealpha" => "preserveAlpha",
        "preserveaspectratio" => "preserveAspectRatio",
        "primitiveunits" => "primitiveUnits",
        "refx" => "refX",
        "refy" => "refY",
        "repeatcount" => "repeatCount",
        "repeatdur" => "repeatDur",
        "requiredextensions" => "requiredExtensions",
        "requiredfeatures" => "requiredFeatures",
        "specularconstant" => "specularConstant",
        "specularexponent" => "specularExponent",
        "spreadmethod" => "spreadMethod",
        "startoffset" => "startOffset",
        "stddeviation" => "stdDeviation",
        "stitchtiles" => "stitchTiles",
        "surfacescale" => "surfaceScale",
        "systemlanguage" => "systemLanguage",
        "tablevalues" => "tableValues",
        "targetx" => "targetX",
        "targety" => "targetY",
        "textlength" => "textLength",
        "viewbox" => "viewBox",
        "viewtarget" => "viewTarget",
        "xchannelselector" => "xChannelSelector",
        "ychannelselector" => "yChannelSelector",
        "zoomandpan" => "zoomAndPan",
        _ => name,
    }
}

/// Adjust `MathML` attribute names.
fn adjust_mathml_attributes(name: &str) -> &str {
    match name {
        "definitionurl" => "definitionURL",
        _ => name,
    }
}

/// Parse a foreign attribute name into (prefix, `local_name`, namespace).
fn parse_foreign_attr(name: &str) -> (Option<&str>, &str, Option<&str>) {
    match name {
        "xlink:actuate" | "xlink:arcrole" | "xlink:href" | "xlink:role" | "xlink:show"
        | "xlink:title" | "xlink:type" => {
            let local = &name[6..]; // skip "xlink:"
            (Some("xlink"), local, Some("http://www.w3.org/1999/xlink"))
        }
        "xml:lang" | "xml:space" => {
            let local = &name[4..]; // skip "xml:"
            (
                Some("xml"),
                local,
                Some("http://www.w3.org/XML/1998/namespace"),
            )
        }
        "xmlns" => (None, "xmlns", Some("http://www.w3.org/2000/xmlns/")),
        "xmlns:xlink" => (
            Some("xmlns"),
            "xlink",
            Some("http://www.w3.org/2000/xmlns/"),
        ),
        _ => (None, name, None),
    }
}

// ---------------------------------------------------------------------------
// TreeBuilder
// ---------------------------------------------------------------------------

/// The HTML5 tree builder state machine.
#[allow(clippy::struct_excessive_bools)]
struct TreeBuilder<'a> {
    tokenizer: Tokenizer<'a>,
    doc: Document,
    mode: InsertionMode,
    original_mode: InsertionMode,
    open_elements: Vec<StackEntry>,
    active_formatting: Vec<FormatEntry>,
    head_pointer: Option<NodeId>,
    form_pointer: Option<NodeId>,
    #[allow(dead_code)]
    scripting: bool,
    frameset_ok: bool,
    foster_parenting: bool,
    template_modes: Vec<InsertionMode>,
    pending_table_chars: Vec<char>,
    /// When true, the next `Character('\n')` token is dropped (leading newline
    /// stripping after `<pre>`, `<listing>`, and `<textarea>`).
    skip_next_lf: bool,
    quirks_mode: QuirksMode,
    /// For fragment parsing: the context element name and namespace.
    fragment_context: Option<(String, Namespace)>,
    #[allow(dead_code)]
    errors: Vec<TreeBuildError>,
}

impl<'a> TreeBuilder<'a> {
    fn new(tokenizer: Tokenizer<'a>, options: &Html5ParseOptions) -> Self {
        let fragment_context = options.fragment_context.as_ref().map(|ctx| {
            // Parse "svg elementname" / "math elementname" / "elementname"
            if let Some(name) = ctx.strip_prefix("svg ") {
                (name.to_string(), Namespace::Svg)
            } else if let Some(name) = ctx.strip_prefix("math ") {
                (name.to_string(), Namespace::MathMl)
            } else {
                (ctx.clone(), Namespace::Html)
            }
        });
        Self {
            tokenizer,
            doc: Document::new(),
            mode: InsertionMode::Initial,
            original_mode: InsertionMode::Initial,
            open_elements: Vec::new(),
            active_formatting: Vec::new(),
            head_pointer: None,
            form_pointer: None,
            scripting: options.scripting,
            frameset_ok: true,
            foster_parenting: false,
            template_modes: Vec::new(),
            pending_table_chars: Vec::new(),
            skip_next_lf: false,
            quirks_mode: QuirksMode::NoQuirks,
            fragment_context,
            errors: Vec::new(),
        }
    }

    // -----------------------------------------------------------------------
    // Main loop
    // -----------------------------------------------------------------------

    /// Initialize the parser for fragment parsing (WHATWG §13.2.1).
    fn initialize_fragment(&mut self) {
        let Some((ref ctx_name, ctx_ns)) = self.fragment_context.clone() else {
            return;
        };

        // Step 5: Create a root html element, append to document, push onto stack.
        let html_id = self.doc.create_node(NodeKind::Element {
            name: "html".to_string(),
            prefix: None,
            namespace: None,
            attributes: Vec::new(),
        });
        self.doc.append_child(self.doc.root(), html_id);
        self.open_elements.push(StackEntry {
            node_id: html_id,
            name: "html".to_string(),
            ns: Namespace::Html,
            is_html_integration: false,
        });

        // Step 6: If context is template, push InTemplate onto template modes.
        if ctx_name == "template" && ctx_ns == Namespace::Html {
            self.template_modes.push(InsertionMode::InTemplate);
        }

        // Step 8: Set tokenizer state based on context element.
        if ctx_ns == Namespace::Html {
            match ctx_name.as_str() {
                "title" | "textarea" => {
                    self.tokenizer.set_state(State::RcData);
                    self.tokenizer.set_last_start_tag(ctx_name);
                }
                "style" | "xmp" | "iframe" | "noembed" | "noframes" => {
                    self.tokenizer.set_state(State::RawText);
                    self.tokenizer.set_last_start_tag(ctx_name);
                }
                "script" => {
                    self.tokenizer.set_state(State::ScriptData);
                }
                "noscript" if self.scripting => {
                    self.tokenizer.set_state(State::RawText);
                    self.tokenizer.set_last_start_tag(ctx_name);
                }
                "plaintext" => {
                    self.tokenizer.set_state(State::Plaintext);
                }
                _ => {}
            }
        }

        // For foreign (SVG/MathML) context elements, push the context
        // element onto the stack so that tokens are processed in foreign
        // content mode. The element is appended to the html root so that
        // child insertion works, but fragment serialization returns its
        // children (not the context element itself).
        if ctx_ns != Namespace::Html {
            let ns_uri = Some(ctx_ns.uri().to_string());
            let ctx_id = self.doc.create_node(NodeKind::Element {
                name: ctx_name.clone(),
                prefix: None,
                namespace: ns_uri,
                attributes: Vec::new(),
            });
            self.doc.append_child(html_id, ctx_id);

            let is_html_integration = ctx_ns == Namespace::Svg
                && matches!(ctx_name.as_str(), "foreignObject" | "desc" | "title");
            self.open_elements.push(StackEntry {
                node_id: ctx_id,
                name: ctx_name.clone(),
                ns: ctx_ns,
                is_html_integration,
            });
        }

        // Step 10: Reset the insertion mode appropriately.
        // (reset_insertion_mode uses fragment_context for the "last" node case)
        self.reset_insertion_mode();

        // Step 12: Set frameset_ok to false.
        self.frameset_ok = false;
    }

    fn run(&mut self) {
        if self.fragment_context.is_some() {
            self.initialize_fragment();
        }
        loop {
            // Inform the tokenizer whether the adjusted current node is in
            // a foreign namespace (controls CDATA section handling).
            let in_foreign = self
                .open_elements
                .last()
                .is_some_and(|el| el.ns != Namespace::Html);
            self.tokenizer.set_allow_cdata(in_foreign);
            let token = self.tokenizer.next_token();
            // Leading newline stripping after <pre>, <listing>, <textarea>.
            if self.skip_next_lf {
                self.skip_next_lf = false;
                if token == Token::Character('\n') {
                    continue;
                }
            }

            // Fast path: batch consecutive non-null Character tokens in InBody
            // mode when there's no foreign content. This avoids per-character
            // overhead from process_token dispatch and insertion point lookups.
            if self.mode == InsertionMode::InBody && !in_foreign {
                if let Token::Character(c) = token {
                    if c != '\0' {
                        let mut buf = String::new();
                        if !is_ascii_whitespace(c) {
                            self.frameset_ok = false;
                        }
                        buf.push(c);
                        // Drain consecutive characters from the pending queue.
                        loop {
                            let next = self.tokenizer.next_token();
                            if let Token::Character(c2) = next {
                                if c2 == '\0' {
                                    // Null in body: parse error, ignored per spec.
                                } else {
                                    if !is_ascii_whitespace(c2) {
                                        self.frameset_ok = false;
                                    }
                                    buf.push(c2);
                                }
                            } else {
                                // Non-character token: insert buffered text, then
                                // process this token normally.
                                if !buf.is_empty() {
                                    self.reconstruct_formatting();
                                    self.insert_characters(&buf);
                                }
                                if next == Token::Eof {
                                    self.process_token(next);
                                    self.populate_selectedcontent();
                                    return;
                                }
                                // Re-check foreign state before processing next.
                                let in_foreign_now = self
                                    .open_elements
                                    .last()
                                    .is_some_and(|el| el.ns != Namespace::Html);
                                self.tokenizer.set_allow_cdata(in_foreign_now);
                                self.process_token(next);
                                break;
                            }
                        }
                        continue;
                    }
                }
            }

            let is_eof = token == Token::Eof;
            self.process_token(token);
            if is_eof {
                break;
            }
        }
        self.populate_selectedcontent();
    }

    /// Populate `<selectedcontent>` elements inside `<select>` by cloning
    /// the content of the selected (or first) `<option>` into them.
    fn populate_selectedcontent(&mut self) {
        // Collect all selectedcontent elements
        let all_nodes: Vec<NodeId> = self.doc.descendants(self.doc.root()).collect();
        let mut selectedcontent_nodes: Vec<NodeId> = Vec::new();
        for &nid in &all_nodes {
            if let NodeKind::Element { ref name, .. } = self.doc.node(nid).kind {
                if name == "selectedcontent" {
                    selectedcontent_nodes.push(nid);
                }
            }
        }

        for sc_id in selectedcontent_nodes {
            // Walk up to find the containing <select>
            let mut select_id = None;
            let mut ancestor = self.doc.parent(sc_id);
            while let Some(a) = ancestor {
                if let NodeKind::Element { ref name, .. } = self.doc.node(a).kind {
                    if name == "select" {
                        select_id = Some(a);
                        break;
                    }
                }
                ancestor = self.doc.parent(a);
            }
            let Some(select_id) = select_id else {
                continue;
            };

            // Find the selected option (or first option) inside the select
            let option_children: Vec<NodeId> = self.doc.descendants(select_id).collect();
            let mut first_option: Option<NodeId> = None;
            let mut selected_option: Option<NodeId> = None;
            for &nid in &option_children {
                if let NodeKind::Element {
                    ref name,
                    ref attributes,
                    ..
                } = self.doc.node(nid).kind
                {
                    if name == "option" {
                        if first_option.is_none() {
                            first_option = Some(nid);
                        }
                        if attributes.iter().any(|a| a.name == "selected") {
                            selected_option = Some(nid);
                        }
                    }
                }
            }

            let source = selected_option.or(first_option);
            let Some(source) = source else {
                continue;
            };

            // Clone all children of the source option into selectedcontent
            let children: Vec<NodeId> = self.doc.children(source).collect();
            for child in children {
                self.deep_clone_into(child, sc_id);
            }
        }
    }

    /// Deep-clone a node and all its descendants, appending the clone to `parent`.
    fn deep_clone_into(&mut self, source: NodeId, parent: NodeId) {
        let kind = self.doc.node(source).kind.clone();
        let clone_id = self.doc.create_node(kind);
        self.doc.append_child(parent, clone_id);
        let children: Vec<NodeId> = self.doc.children(source).collect();
        for child in children {
            self.deep_clone_into(child, clone_id);
        }
    }

    #[allow(clippy::too_many_lines)]
    fn process_token(&mut self, token: Token) {
        // Determine whether to use normal insertion mode rules or foreign content.
        // Per WHATWG §13.2.6.
        if self.should_use_foreign_content_rules(&token) {
            self.handle_foreign_content(token);
            return;
        }

        match self.mode {
            InsertionMode::Initial => self.handle_initial(token),
            InsertionMode::BeforeHtml => self.handle_before_html(token),
            InsertionMode::BeforeHead => self.handle_before_head(token),
            InsertionMode::InHead => self.handle_in_head(token),
            InsertionMode::InHeadNoscript => self.handle_in_head_noscript(token),
            InsertionMode::AfterHead => self.handle_after_head(token),
            InsertionMode::InBody => self.handle_in_body(token),
            InsertionMode::Text => self.handle_text(token),
            InsertionMode::InTable => self.handle_in_table(token),
            InsertionMode::InTableText => self.handle_in_table_text(token),
            InsertionMode::InCaption => self.handle_in_caption(token),
            InsertionMode::InColumnGroup => self.handle_in_column_group(token),
            InsertionMode::InTableBody => self.handle_in_table_body(token),
            InsertionMode::InRow => self.handle_in_row(token),
            InsertionMode::InCell => self.handle_in_cell(token),
            InsertionMode::InSelect => self.handle_in_select(token),
            InsertionMode::InSelectInTable => self.handle_in_select_in_table(token),
            InsertionMode::InTemplate => self.handle_in_template(token),
            InsertionMode::AfterBody => self.handle_after_body(token),
            InsertionMode::InFrameset => self.handle_in_frameset(token),
            InsertionMode::AfterFrameset => self.handle_after_frameset(token),
            InsertionMode::AfterAfterBody => self.handle_after_after_body(token),
            InsertionMode::AfterAfterFrameset => self.handle_after_after_frameset(token),
        }
    }

    // -----------------------------------------------------------------------
    // Foreign content dispatcher (WHATWG §13.2.6)
    // -----------------------------------------------------------------------

    fn is_mathml_text_integration_point(entry: &StackEntry) -> bool {
        entry.ns == Namespace::MathMl
            && matches!(entry.name.as_str(), "mi" | "mo" | "mn" | "ms" | "mtext")
    }

    fn is_html_integration_point(entry: &StackEntry) -> bool {
        if entry.ns == Namespace::Svg
            && matches!(entry.name.as_str(), "foreignObject" | "desc" | "title")
        {
            return true;
        }
        // MathML annotation-xml with encoding text/html or application/xhtml+xml
        entry.is_html_integration
    }

    fn should_use_foreign_content_rules(&self, token: &Token) -> bool {
        let Some(cur) = self.open_elements.last() else {
            return false;
        };

        // If adjusted current node is in HTML namespace, use normal rules.
        if cur.ns == Namespace::Html {
            return false;
        }

        // MathML text integration point: start tags (except mglyph/malignmark)
        // and character tokens use normal rules.
        if Self::is_mathml_text_integration_point(cur) {
            match token {
                Token::StartTag { name, .. } if name != "mglyph" && name != "malignmark" => {
                    return false;
                }
                Token::Character(_) => return false,
                _ => {}
            }
        }

        // MathML annotation-xml + start tag "svg" → normal rules
        if cur.ns == Namespace::MathMl && cur.name == "annotation-xml" {
            if let Token::StartTag { name, .. } = token {
                if name == "svg" {
                    return false;
                }
            }
        }

        // HTML integration point: start tags and character tokens use normal rules.
        if Self::is_html_integration_point(cur) {
            match token {
                Token::StartTag { .. } | Token::Character(_) => return false,
                _ => {}
            }
        }

        // EOF always uses normal rules.
        if *token == Token::Eof {
            return false;
        }

        true
    }

    #[allow(clippy::too_many_lines)]
    fn handle_foreign_content(&mut self, token: Token) {
        match token {
            Token::Character('\0') => {
                self.insert_character('\u{FFFD}');
            }
            Token::Character(c) if is_ascii_whitespace(c) => {
                self.insert_character(c);
            }
            Token::Character(c) => {
                self.insert_character(c);
                self.frameset_ok = false;
            }
            Token::Comment(data) => {
                self.insert_comment(&data);
            }
            Token::Doctype { .. } | Token::Eof => {
                // Parse error, ignore.
            }
            Token::StartTag {
                ref name,
                ref attributes,
                ..
            } if is_foreign_breakout_tag(name)
                || (name == "font"
                    && attributes
                        .iter()
                        .any(|a| matches!(a.name.as_str(), "color" | "face" | "size"))) =>
            {
                // Parse error. Pop until MathML text integration point,
                // HTML integration point, or HTML namespace element.
                while let Some(top) = self.open_elements.last() {
                    if top.ns == Namespace::Html
                        || Self::is_mathml_text_integration_point(top)
                        || Self::is_html_integration_point(top)
                    {
                        break;
                    }
                    // In fragment mode, don't pop the context element.
                    if self.open_elements.len() <= 2
                        && self
                            .fragment_context
                            .as_ref()
                            .is_some_and(|(_, ns)| *ns != Namespace::Html)
                    {
                        break;
                    }
                    self.open_elements.pop();
                }
                // Use dispatch_to_current_mode to avoid re-entering
                // foreign content handling when the context is foreign.
                self.dispatch_to_current_mode(token);
            }
            Token::StartTag {
                name,
                attributes,
                self_closing,
            } => {
                // Any other start tag in foreign content.
                let cur_ns = self.open_elements.last().map_or(Namespace::Html, |e| e.ns);

                let (adjusted_name, adjusted_attrs, ns) = match cur_ns {
                    Namespace::MathMl => {
                        let attrs: Vec<tokenizer::Attribute> = attributes
                            .iter()
                            .map(|a| tokenizer::Attribute {
                                name: adjust_mathml_attributes(&a.name).to_string(),
                                value: a.value.clone(),
                            })
                            .collect();
                        (name, attrs, Namespace::MathMl)
                    }
                    Namespace::Svg => {
                        let tag = adjust_svg_tag_name(&name);
                        let attrs: Vec<tokenizer::Attribute> = attributes
                            .iter()
                            .map(|a| tokenizer::Attribute {
                                name: adjust_svg_attributes(&a.name).to_string(),
                                value: a.value.clone(),
                            })
                            .collect();
                        (tag, attrs, Namespace::Svg)
                    }
                    Namespace::Html => (name, attributes, Namespace::Html),
                };

                self.insert_foreign_element(&adjusted_name, &adjusted_attrs, ns);

                if self_closing {
                    self.open_elements.pop();
                }
            }
            Token::EndTag { ref name } if name == "br" || name == "p" => {
                // Per spec §13.2.6.5: parse error. Pop until we reach an
                // HTML namespace element, MathML text integration point, or
                // HTML integration point; then reprocess as "in body".
                // In fragment mode, never pop below the context element.
                let min_stack = if self
                    .fragment_context
                    .as_ref()
                    .is_some_and(|(_, ns)| *ns != Namespace::Html)
                {
                    2
                } else {
                    1
                };
                while self.open_elements.len() > min_stack {
                    let Some(top) = self.open_elements.last() else {
                        break;
                    };
                    if top.ns == Namespace::Html
                        || Self::is_mathml_text_integration_point(top)
                        || Self::is_html_integration_point(top)
                    {
                        break;
                    }
                    self.open_elements.pop();
                }
                self.dispatch_to_current_mode(token);
            }
            Token::EndTag { name } => {
                // Any other end tag in foreign content.
                self.handle_foreign_end_tag(&name);
            }
        }
    }

    fn handle_foreign_end_tag(&mut self, tag_name: &str) {
        if self.open_elements.is_empty() {
            return;
        }

        // In fragment parsing, never pop below the initial stack
        // (html element, or html + context element for foreign contexts).
        let min_idx = if self.fragment_context.is_some() {
            // html is at 0; for foreign contexts the context element is at 1.
            if self
                .fragment_context
                .as_ref()
                .is_some_and(|(_, ns)| *ns != Namespace::Html)
            {
                2
            } else {
                1
            }
        } else {
            0
        };

        let mut node_idx = self.open_elements.len() - 1;

        loop {
            let node = &self.open_elements[node_idx];

            if node.ns == Namespace::Html {
                // Process using the rules for the current insertion mode
                // (not process_token, to avoid re-entering foreign content).
                self.dispatch_to_current_mode(Token::EndTag {
                    name: tag_name.to_string(),
                });
                return;
            }

            if node.name.eq_ignore_ascii_case(tag_name) {
                // Don't pop the context element or below it.
                let pop_to = node_idx.max(min_idx);
                while self.open_elements.len() > pop_to {
                    self.open_elements.pop();
                }
                return;
            }

            if node_idx <= min_idx {
                return;
            }
            node_idx -= 1;
        }
    }

    /// Dispatch a token directly to the current insertion mode handler,
    /// bypassing the foreign content check.
    fn dispatch_to_current_mode(&mut self, token: Token) {
        match self.mode {
            InsertionMode::Initial => self.handle_initial(token),
            InsertionMode::BeforeHtml => self.handle_before_html(token),
            InsertionMode::BeforeHead => self.handle_before_head(token),
            InsertionMode::InHead => self.handle_in_head(token),
            InsertionMode::InHeadNoscript => self.handle_in_head_noscript(token),
            InsertionMode::AfterHead => self.handle_after_head(token),
            InsertionMode::InBody => self.handle_in_body(token),
            InsertionMode::Text => self.handle_text(token),
            InsertionMode::InTable => self.handle_in_table(token),
            InsertionMode::InTableText => self.handle_in_table_text(token),
            InsertionMode::InCaption => self.handle_in_caption(token),
            InsertionMode::InColumnGroup => self.handle_in_column_group(token),
            InsertionMode::InTableBody => self.handle_in_table_body(token),
            InsertionMode::InRow => self.handle_in_row(token),
            InsertionMode::InCell => self.handle_in_cell(token),
            InsertionMode::InSelect => self.handle_in_select(token),
            InsertionMode::InSelectInTable => self.handle_in_select_in_table(token),
            InsertionMode::InTemplate => self.handle_in_template(token),
            InsertionMode::AfterBody => self.handle_after_body(token),
            InsertionMode::InFrameset => self.handle_in_frameset(token),
            InsertionMode::AfterFrameset => self.handle_after_frameset(token),
            InsertionMode::AfterAfterBody => self.handle_after_after_body(token),
            InsertionMode::AfterAfterFrameset => self.handle_after_after_frameset(token),
        }
    }

    fn insert_foreign_element(
        &mut self,
        name: &str,
        attrs: &[tokenizer::Attribute],
        ns: Namespace,
    ) -> NodeId {
        let tree_attrs: Vec<crate::tree::Attribute> = attrs
            .iter()
            .map(|a| {
                let (prefix, local_name, attr_ns) = parse_foreign_attr(&a.name);
                crate::tree::Attribute {
                    name: local_name.to_string(),
                    value: a.value.clone(),
                    prefix: prefix.map(String::from),
                    namespace: attr_ns.map(String::from),
                    raw_value: None,
                }
            })
            .collect();

        let namespace = if ns == Namespace::Html {
            None
        } else {
            Some(ns.uri().to_string())
        };

        let id_value = tree_attrs.iter().find_map(|a| {
            if a.name == "id" {
                Some(a.value.clone())
            } else {
                None
            }
        });

        let node_id = self.doc.create_node(NodeKind::Element {
            name: name.to_string(),
            prefix: None,
            namespace,
            attributes: tree_attrs,
        });

        if let Some(id_val) = id_value {
            self.doc.set_id(&id_val, node_id);
        }

        // Detect HTML integration point: MathML annotation-xml with
        // encoding="text/html" or "application/xhtml+xml".
        let is_html_integration = ns == Namespace::MathMl
            && name == "annotation-xml"
            && attrs.iter().any(|a| {
                a.name.eq_ignore_ascii_case("encoding")
                    && (a.value.eq_ignore_ascii_case("text/html")
                        || a.value.eq_ignore_ascii_case("application/xhtml+xml"))
            });

        let (parent, before) = self.appropriate_insertion_point();
        self.insert_node_at(node_id, parent, before);
        self.open_elements.push(StackEntry {
            node_id,
            name: name.to_string(),
            ns,
            is_html_integration,
        });
        node_id
    }

    // -----------------------------------------------------------------------
    // Stack / scope helpers
    // -----------------------------------------------------------------------

    fn current_node(&self) -> Option<NodeId> {
        self.open_elements.last().map(|e| e.node_id)
    }

    fn current_node_name(&self) -> &str {
        self.open_elements.last().map_or("", |e| e.name.as_str())
    }

    fn element_in_scope_impl(&self, target: &str, scope_fn: fn(&str, Namespace) -> bool) -> bool {
        for entry in self.open_elements.iter().rev() {
            if entry.name == target && entry.ns == Namespace::Html {
                return true;
            }
            if scope_fn(&entry.name, entry.ns) {
                return false;
            }
        }
        false
    }

    fn element_in_scope(&self, target: &str) -> bool {
        self.element_in_scope_impl(target, is_scope_element)
    }

    fn element_in_list_item_scope(&self, target: &str) -> bool {
        self.element_in_scope_impl(target, is_list_item_scope_element)
    }

    fn element_in_button_scope(&self, target: &str) -> bool {
        self.element_in_scope_impl(target, is_button_scope_element)
    }

    fn element_in_table_scope(&self, target: &str) -> bool {
        self.element_in_scope_impl(target, is_table_scope_element)
    }

    fn element_in_select_scope(&self, target: &str) -> bool {
        self.element_in_scope_impl(target, is_select_scope_element)
    }

    // -----------------------------------------------------------------------
    // Insertion helpers
    // -----------------------------------------------------------------------

    fn appropriate_insertion_point(&self) -> (NodeId, Option<NodeId>) {
        self.appropriate_insertion_point_with_override(None)
    }

    fn appropriate_insertion_point_with_override(
        &self,
        override_target: Option<NodeId>,
    ) -> (NodeId, Option<NodeId>) {
        let target = override_target
            .unwrap_or_else(|| self.current_node().unwrap_or_else(|| self.doc.root()));

        // Per WHATWG spec §13.2.6.1: foster parenting only applies when the
        // target is a table, tbody, tfoot, thead, or tr element.
        if self.foster_parenting {
            let target_name = self
                .open_elements
                .iter()
                .find(|e| e.node_id == target)
                .map_or("", |e| e.name.as_str());
            if matches!(target_name, "table" | "tbody" | "tfoot" | "thead" | "tr") {
                // Find the last table and last template in the stack.
                let mut last_table: Option<usize> = None;
                let mut last_template: Option<usize> = None;
                for i in (0..self.open_elements.len()).rev() {
                    if self.open_elements[i].name == "table"
                        && self.open_elements[i].ns == Namespace::Html
                        && last_table.is_none()
                    {
                        last_table = Some(i);
                    }
                    if self.open_elements[i].name == "template"
                        && self.open_elements[i].ns == Namespace::Html
                        && last_template.is_none()
                    {
                        last_template = Some(i);
                    }
                }

                // If template comes after table (or no table), insert
                // inside the template element (its content).
                if let Some(tmpl_idx) = last_template {
                    if last_table.is_none() || tmpl_idx > last_table.unwrap_or(0) {
                        return (self.open_elements[tmpl_idx].node_id, None);
                    }
                }

                if let Some(table_idx) = last_table {
                    if let Some(parent) = self.doc.parent(self.open_elements[table_idx].node_id) {
                        return (parent, Some(self.open_elements[table_idx].node_id));
                    }
                    // If table has no parent, use the element before it in the stack
                    if table_idx > 0 {
                        return (self.open_elements[table_idx - 1].node_id, None);
                    }
                }

                // No table or template — fall back to first element
                if !self.open_elements.is_empty() {
                    return (self.open_elements[0].node_id, None);
                }
            }
        }
        (target, None)
    }

    fn insert_node_at(&mut self, node_id: NodeId, parent: NodeId, before: Option<NodeId>) {
        if let Some(ref_node) = before {
            self.doc.insert_before(ref_node, node_id);
        } else {
            self.doc.append_child(parent, node_id);
        }
    }

    fn create_element_for_token(
        &mut self,
        name: &str,
        attrs: &[tokenizer::Attribute],
        ns: Namespace,
    ) -> NodeId {
        let tree_attrs: Vec<crate::tree::Attribute> = attrs
            .iter()
            .map(|a| crate::tree::Attribute {
                name: a.name.clone(),
                value: a.value.clone(),
                prefix: None,
                namespace: None,
                raw_value: None,
            })
            .collect();

        let namespace = if ns == Namespace::Html {
            None
        } else {
            Some(ns.uri().to_string())
        };

        self.doc.create_node(NodeKind::Element {
            name: name.to_string(),
            prefix: None,
            namespace,
            attributes: tree_attrs,
        })
    }

    fn insert_element(
        &mut self,
        name: &str,
        attrs: &[tokenizer::Attribute],
        ns: Namespace,
    ) -> NodeId {
        let node_id = self.create_element_for_token(name, attrs, ns);
        let (parent, before) = self.appropriate_insertion_point();
        self.insert_node_at(node_id, parent, before);
        self.open_elements.push(StackEntry {
            node_id,
            name: name.to_string(),
            ns,
            is_html_integration: false,
        });
        node_id
    }

    fn insert_html_element(&mut self, name: &str, attrs: &[tokenizer::Attribute]) -> NodeId {
        self.insert_element(name, attrs, Namespace::Html)
    }

    fn insert_character(&mut self, c: char) {
        let (parent, before) = self.appropriate_insertion_point();

        // Try to append to existing text node — either the last child of
        // parent (normal case) or the previous sibling of the reference node
        // (foster-parenting case).
        let adjacent_text = if let Some(ref_node) = before {
            self.doc.prev_sibling(ref_node)
        } else {
            self.doc.last_child(parent)
        };
        if let Some(text_node) = adjacent_text {
            if let NodeKind::Text { ref mut content } = &mut self.doc.node_mut(text_node).kind {
                content.push(c);
                return;
            }
        }

        let text_id = self.doc.create_node(NodeKind::Text {
            content: c.to_string(),
        });
        self.insert_node_at(text_id, parent, before);
    }

    /// Append a string of characters to the current insertion point.
    ///
    /// This is an optimization over calling `insert_character` per-char:
    /// it computes the insertion point once and appends the whole string.
    fn insert_characters(&mut self, s: &str) {
        let (parent, before) = self.appropriate_insertion_point();
        let adjacent_text = if let Some(ref_node) = before {
            self.doc.prev_sibling(ref_node)
        } else {
            self.doc.last_child(parent)
        };
        if let Some(text_node) = adjacent_text {
            if let NodeKind::Text { ref mut content } = &mut self.doc.node_mut(text_node).kind {
                content.push_str(s);
                return;
            }
        }
        let text_id = self.doc.create_node(NodeKind::Text {
            content: s.to_string(),
        });
        self.insert_node_at(text_id, parent, before);
    }

    fn insert_comment(&mut self, data: &str) {
        let (parent, before) = self.appropriate_insertion_point();
        let comment_id = self.doc.create_node(NodeKind::Comment {
            content: data.to_string(),
        });
        self.insert_node_at(comment_id, parent, before);
    }

    fn insert_comment_at_document(&mut self, data: &str) {
        let doc_root = self.doc.root();
        let comment_id = self.doc.create_node(NodeKind::Comment {
            content: data.to_string(),
        });
        self.doc.append_child(doc_root, comment_id);
    }

    // -----------------------------------------------------------------------
    // Implied end tags
    // -----------------------------------------------------------------------

    fn generate_implied_end_tags(&mut self, exclude: Option<&str>) {
        loop {
            let name = self.current_node_name().to_string();
            if matches!(
                name.as_str(),
                "dd" | "dt" | "li" | "optgroup" | "option" | "p" | "rb" | "rp" | "rt" | "rtc"
            ) && exclude.map_or(true, |ex| ex != name)
            {
                self.open_elements.pop();
            } else {
                break;
            }
        }
    }

    fn generate_all_implied_end_tags(&mut self) {
        loop {
            let name = self.current_node_name().to_string();
            if matches!(
                name.as_str(),
                "dd" | "dt"
                    | "li"
                    | "optgroup"
                    | "option"
                    | "p"
                    | "rb"
                    | "rp"
                    | "rt"
                    | "rtc"
                    | "tbody"
                    | "td"
                    | "tfoot"
                    | "th"
                    | "thead"
                    | "tr"
                    | "caption"
                    | "colgroup"
            ) {
                self.open_elements.pop();
            } else {
                break;
            }
        }
    }

    fn close_p_element(&mut self) {
        self.generate_implied_end_tags(Some("p"));
        // Pop until p
        while let Some(entry) = self.open_elements.pop() {
            if entry.name == "p" {
                break;
            }
        }
    }

    // -----------------------------------------------------------------------
    // Active formatting list
    // -----------------------------------------------------------------------

    fn push_formatting(&mut self, node_id: NodeId, name: &str, attrs: &[tokenizer::Attribute]) {
        // Noah's Ark clause: if there are already 3 entries with the same
        // tag name and attributes before the last marker, remove the earliest.
        let mut count = 0;
        let mut earliest_idx = None;
        for (i, entry) in self.active_formatting.iter().enumerate().rev() {
            match entry {
                FormatEntry::Marker => break,
                FormatEntry::Element {
                    name: n, attrs: a, ..
                } if n == name && a == attrs => {
                    count += 1;
                    earliest_idx = Some(i);
                }
                FormatEntry::Element { .. } => {}
            }
        }
        if count >= 3 {
            if let Some(idx) = earliest_idx {
                self.active_formatting.remove(idx);
            }
        }
        self.active_formatting.push(FormatEntry::Element {
            node_id,
            name: name.to_string(),
            attrs: attrs.to_vec(),
        });
    }

    fn push_formatting_marker(&mut self) {
        self.active_formatting.push(FormatEntry::Marker);
    }

    fn clear_formatting_to_marker(&mut self) {
        while let Some(entry) = self.active_formatting.pop() {
            if matches!(entry, FormatEntry::Marker) {
                break;
            }
        }
    }

    fn reconstruct_formatting(&mut self) {
        if self.active_formatting.is_empty() {
            return;
        }

        // If the last entry is a marker or already on the stack, nothing to do.
        if let Some(last) = self.active_formatting.last() {
            match last {
                FormatEntry::Marker => return,
                FormatEntry::Element { node_id, .. } => {
                    if self.open_elements.iter().any(|e| e.node_id == *node_id) {
                        return;
                    }
                }
            }
        }

        // Walk backwards to find the first entry that IS on the stack or is a marker.
        let mut i = self.active_formatting.len() - 1;
        loop {
            if i == 0 {
                break;
            }
            i -= 1;
            match &self.active_formatting[i] {
                FormatEntry::Marker => {
                    i += 1;
                    break;
                }
                FormatEntry::Element { node_id, .. } => {
                    if self.open_elements.iter().any(|e| e.node_id == *node_id) {
                        i += 1;
                        break;
                    }
                }
            }
        }

        // Now walk forward from i, creating new elements.
        while i < self.active_formatting.len() {
            let (name, attrs) = match &self.active_formatting[i] {
                FormatEntry::Element { name, attrs, .. } => (name.clone(), attrs.clone()),
                FormatEntry::Marker => {
                    i += 1;
                    continue;
                }
            };

            let new_id = self.insert_html_element(&name, &attrs);
            self.active_formatting[i] = FormatEntry::Element {
                node_id: new_id,
                name,
                attrs,
            };
            i += 1;
        }
    }

    // -----------------------------------------------------------------------
    // Adoption agency algorithm (WHATWG 13.2.6.4.7)
    // -----------------------------------------------------------------------

    #[allow(clippy::too_many_lines)]
    fn adoption_agency(&mut self, tag_name: &str) {
        // Step 1: If current node is an HTML element with tag name equal to
        // the token's tag name, and the current node is not in the active
        // formatting list, just pop it.
        if let Some(cur) = self.open_elements.last() {
            if cur.name == tag_name && cur.ns == Namespace::Html {
                let cur_id = cur.node_id;
                let in_formatting = self.active_formatting.iter().any(
                    |e| matches!(e, FormatEntry::Element { node_id, .. } if *node_id == cur_id),
                );
                if !in_formatting {
                    self.open_elements.pop();
                    return;
                }
            }
        }

        // Outer loop (max 8 iterations)
        for _ in 0..8 {
            // Step 4: Find the formatting element — the last entry in the
            // active formatting list that has tag name equal to tag_name and
            // that is before the last marker (or the start of the list).
            let fmt_idx = {
                let mut found = None;
                for (i, entry) in self.active_formatting.iter().enumerate().rev() {
                    match entry {
                        FormatEntry::Marker => break,
                        FormatEntry::Element { name, .. } if name == tag_name => {
                            found = Some(i);
                            break;
                        }
                        FormatEntry::Element { .. } => {}
                    }
                }
                found
            };

            let Some(fmt_idx) = fmt_idx else {
                // No formatting element found; process as "any other end tag".
                self.handle_any_other_end_tag(tag_name);
                return;
            };

            let FormatEntry::Element {
                node_id: fmt_node_id,
                name: ref fmt_name,
                attrs: ref fmt_attrs,
            } = self.active_formatting[fmt_idx]
            else {
                return;
            };
            let fmt_name = fmt_name.clone();
            let fmt_attrs = fmt_attrs.clone();

            // Step 5: If the formatting element is not on the stack of open
            // elements, remove it from the formatting list and return.
            let Some(stack_idx) = self
                .open_elements
                .iter()
                .position(|e| e.node_id == fmt_node_id)
            else {
                self.active_formatting.remove(fmt_idx);
                return;
            };

            // Step 6: If the formatting element is not in scope, return.
            if !self.element_in_scope(&fmt_name) {
                return;
            }

            // Step 8: Find the furthest block.
            let furthest_block_idx = self.open_elements[stack_idx + 1..]
                .iter()
                .position(|e| is_special_element_ns(&e.name, e.ns))
                .map(|i| i + stack_idx + 1);

            // Step 9: No furthest block → pop to formatting element.
            let Some(furthest_block_idx) = furthest_block_idx else {
                while self.open_elements.len() > stack_idx {
                    self.open_elements.pop();
                }
                self.active_formatting.remove(fmt_idx);
                return;
            };

            // We need to track the furthest block by node_id since indices shift.
            let furthest_block_node_id = self.open_elements[furthest_block_idx].node_id;

            // Step 10-11
            let common_ancestor = self.open_elements[stack_idx - 1].node_id;
            let mut bookmark = fmt_idx;

            // Step 12: inner loop
            let mut node_stack_idx = furthest_block_idx;
            let mut last_node_id = furthest_block_node_id;
            let mut inner_counter = 0u32;

            loop {
                inner_counter += 1;

                // Step 12.2: node = element immediately above node in stack
                if node_stack_idx == 0 {
                    break;
                }
                node_stack_idx -= 1;
                if node_stack_idx <= stack_idx {
                    break;
                }

                let node_id = self.open_elements[node_stack_idx].node_id;

                // Step 12.3: Check if node is in the active formatting list
                let fmt_list_idx = self.active_formatting.iter().position(
                    |e| matches!(e, FormatEntry::Element { node_id: nid, .. } if *nid == node_id),
                );

                // Step 12.4: If not in formatting list, remove from stack
                let Some(fmt_list_idx) = fmt_list_idx else {
                    self.open_elements.remove(node_stack_idx);
                    continue;
                };

                // Step 12.5: If inner counter > 3, remove from formatting list
                if inner_counter > 3 {
                    self.active_formatting.remove(fmt_list_idx);
                    if bookmark > fmt_list_idx {
                        bookmark -= 1;
                    }
                    self.open_elements.remove(node_stack_idx);
                    continue;
                }

                // Step 12.6-7: Create replacement element
                let (old_name, old_attrs) = match &self.active_formatting[fmt_list_idx] {
                    FormatEntry::Element { name, attrs, .. } => (name.clone(), attrs.clone()),
                    FormatEntry::Marker => continue,
                };

                let new_element =
                    self.create_element_for_token(&old_name, &old_attrs, Namespace::Html);

                self.active_formatting[fmt_list_idx] = FormatEntry::Element {
                    node_id: new_element,
                    name: old_name.clone(),
                    attrs: old_attrs,
                };
                self.open_elements[node_stack_idx] = StackEntry {
                    node_id: new_element,
                    name: old_name,
                    ns: Namespace::Html,
                    is_html_integration: false,
                };

                // Step 12.8: If last node was the furthest block, move bookmark
                if last_node_id == furthest_block_node_id {
                    bookmark = fmt_list_idx + 1;
                }

                // Step 12.9: Move last_node to be a child of new_element
                self.doc.detach(last_node_id);
                self.doc.append_child(new_element, last_node_id);
                last_node_id = new_element;
            }

            // Step 13: insert last_node at the appropriate place
            self.doc.detach(last_node_id);
            // Use the appropriate insertion point with common ancestor as the
            // override target. This correctly handles foster parenting when
            // the common ancestor is a table-related element.
            let (parent, before) =
                self.appropriate_insertion_point_with_override(Some(common_ancestor));
            self.insert_node_at(last_node_id, parent, before);

            // Step 14: create a new element for the formatting element
            let new_fmt = self.create_element_for_token(&fmt_name, &fmt_attrs, Namespace::Html);

            // Step 15: move children of the furthest block to the new element
            let fb_id = self
                .open_elements
                .iter()
                .find(|e| e.node_id == furthest_block_node_id)
                .map(|e| e.node_id);
            if let Some(fb_id) = fb_id {
                let children: Vec<NodeId> = self.doc.children(fb_id).collect();
                for child in children {
                    self.doc.detach(child);
                    self.doc.append_child(new_fmt, child);
                }
                // Step 16: append new element to the furthest block
                self.doc.append_child(fb_id, new_fmt);
            }

            // Step 17: remove old formatting element, insert new at bookmark
            if let Some(old_pos) = self.active_formatting.iter().position(
                |e| matches!(e, FormatEntry::Element { node_id, .. } if *node_id == fmt_node_id),
            ) {
                self.active_formatting.remove(old_pos);
                if bookmark > old_pos {
                    bookmark -= 1;
                }
            }
            let bookmark = bookmark.min(self.active_formatting.len());
            self.active_formatting.insert(
                bookmark,
                FormatEntry::Element {
                    node_id: new_fmt,
                    name: fmt_name.clone(),
                    attrs: fmt_attrs.clone(),
                },
            );

            // Step 18: remove old from stack, insert new after furthest block
            if let Some(old_pos) = self
                .open_elements
                .iter()
                .position(|e| e.node_id == fmt_node_id)
            {
                self.open_elements.remove(old_pos);
            }
            if let Some(fb_pos) =
                fb_id.and_then(|fb| self.open_elements.iter().position(|e| e.node_id == fb))
            {
                let insert_pos = (fb_pos + 1).min(self.open_elements.len());
                self.open_elements.insert(
                    insert_pos,
                    StackEntry {
                        node_id: new_fmt,
                        name: fmt_name,
                        ns: Namespace::Html,
                        is_html_integration: false,
                    },
                );
            }
        }
    }

    // -----------------------------------------------------------------------
    // Tokenizer state switching for raw text elements
    // -----------------------------------------------------------------------

    #[allow(dead_code)]
    fn switch_tokenizer_for_raw(&mut self, name: &str) {
        match name {
            "script" => self.tokenizer.set_state(State::ScriptData),
            "style" | "noframes" | "noembed" | "noscript" => {
                self.tokenizer.set_state(State::RawText);
            }
            "textarea" | "title" => {
                self.tokenizer.set_state(State::RcData);
            }
            "plaintext" => self.tokenizer.set_state(State::Plaintext),
            _ => {}
        }
        self.tokenizer.set_last_start_tag(name);
    }

    fn parse_raw_text(&mut self, name: &str, attrs: &[tokenizer::Attribute]) {
        self.insert_html_element(name, attrs);
        self.tokenizer.set_state(State::RawText);
        self.tokenizer.set_last_start_tag(name);
        self.original_mode = self.mode;
        self.mode = InsertionMode::Text;
    }

    fn parse_rcdata(&mut self, name: &str, attrs: &[tokenizer::Attribute]) {
        self.insert_html_element(name, attrs);
        self.tokenizer.set_state(State::RcData);
        self.tokenizer.set_last_start_tag(name);
        self.original_mode = self.mode;
        self.mode = InsertionMode::Text;
    }

    // -----------------------------------------------------------------------
    // Insertion mode handlers
    // -----------------------------------------------------------------------

    fn handle_initial(&mut self, token: Token) {
        match token {
            Token::Character(c) if is_ascii_whitespace(c) => {
                // Ignore
            }
            Token::Comment(data) => {
                self.insert_comment_at_document(&data);
            }
            Token::Doctype {
                name,
                public_id,
                system_id,
                force_quirks,
            } => {
                let doctype_name = name.unwrap_or_default();
                self.quirks_mode = determine_quirks_mode(
                    &doctype_name,
                    public_id.as_deref(),
                    system_id.as_deref(),
                    force_quirks,
                );
                let doctype_id = self.doc.create_node(NodeKind::DocumentType {
                    name: doctype_name,
                    public_id,
                    system_id,
                    internal_subset: None,
                });
                let root = self.doc.root();
                self.doc.append_child(root, doctype_id);
                self.mode = InsertionMode::BeforeHtml;
            }
            _ => {
                // Missing DOCTYPE → quirks mode.
                self.quirks_mode = QuirksMode::Quirks;
                self.mode = InsertionMode::BeforeHtml;
                self.process_token(token);
            }
        }
    }

    fn handle_before_html(&mut self, token: Token) {
        match token {
            Token::Comment(data) => {
                self.insert_comment_at_document(&data);
            }
            Token::Doctype { .. } => { /* ignore */ }
            Token::Character(c) if is_ascii_whitespace(c) => { /* ignore */ }
            Token::StartTag { ref name, .. } if name == "html" => {
                if let Token::StartTag {
                    name, attributes, ..
                } = token
                {
                    let node_id =
                        self.create_element_for_token(&name, &attributes, Namespace::Html);
                    let root = self.doc.root();
                    self.doc.append_child(root, node_id);
                    self.open_elements.push(StackEntry {
                        node_id,
                        name,
                        ns: Namespace::Html,
                        is_html_integration: false,
                    });
                    self.mode = InsertionMode::BeforeHead;
                }
            }
            Token::EndTag { ref name }
                if !matches!(name.as_str(), "head" | "body" | "html" | "br") =>
            {
                // Parse error, ignore
            }
            _ => {
                let node_id = self.create_element_for_token("html", &[], Namespace::Html);
                let root = self.doc.root();
                self.doc.append_child(root, node_id);
                self.open_elements.push(StackEntry {
                    node_id,
                    name: "html".to_string(),
                    ns: Namespace::Html,
                    is_html_integration: false,
                });
                self.mode = InsertionMode::BeforeHead;
                self.process_token(token);
            }
        }
    }

    fn handle_before_head(&mut self, token: Token) {
        match token {
            Token::Character(c) if is_ascii_whitespace(c) => { /* ignore */ }
            Token::Comment(data) => self.insert_comment(&data),
            Token::Doctype { .. } => { /* ignore */ }
            Token::StartTag { ref name, .. } if name == "html" => {
                self.handle_in_body(token);
            }
            Token::StartTag { ref name, .. } if name == "head" => {
                if let Token::StartTag {
                    name, attributes, ..
                } = token
                {
                    let node_id = self.insert_html_element(&name, &attributes);
                    self.head_pointer = Some(node_id);
                    self.mode = InsertionMode::InHead;
                }
            }
            Token::EndTag { ref name }
                if !matches!(name.as_str(), "head" | "body" | "html" | "br") =>
            {
                // Parse error, ignore
            }
            _ => {
                let node_id = self.insert_html_element("head", &[]);
                self.head_pointer = Some(node_id);
                self.mode = InsertionMode::InHead;
                self.process_token(token);
            }
        }
    }

    #[allow(clippy::too_many_lines, clippy::match_same_arms)]
    fn handle_in_head(&mut self, token: Token) {
        match token {
            Token::Character(c) if is_ascii_whitespace(c) => {
                self.insert_character(c);
            }
            Token::Comment(data) => self.insert_comment(&data),
            Token::Doctype { .. } => { /* ignore */ }
            Token::StartTag { ref name, .. } if name == "html" => {
                self.handle_in_body(token);
            }
            Token::StartTag { ref name, .. }
                if matches!(
                    name.as_str(),
                    "base" | "basefont" | "bgsound" | "link" | "meta"
                ) =>
            {
                if let Token::StartTag {
                    name, attributes, ..
                } = token
                {
                    self.insert_html_element(&name, &attributes);
                    self.open_elements.pop(); // void
                }
            }
            Token::StartTag { ref name, .. } if name == "title" => {
                if let Token::StartTag {
                    name, attributes, ..
                } = token
                {
                    self.parse_rcdata(&name, &attributes);
                }
            }
            Token::StartTag { ref name, .. } if name == "noscript" && self.scripting => {
                if let Token::StartTag {
                    name, attributes, ..
                } = token
                {
                    self.parse_raw_text(&name, &attributes);
                }
            }
            Token::StartTag { ref name, .. } if matches!(name.as_str(), "noframes" | "style") => {
                if let Token::StartTag {
                    name, attributes, ..
                } = token
                {
                    self.parse_raw_text(&name, &attributes);
                }
            }
            Token::StartTag { ref name, .. } if name == "noscript" => {
                if let Token::StartTag {
                    name, attributes, ..
                } = token
                {
                    self.insert_html_element(&name, &attributes);
                    self.mode = InsertionMode::InHeadNoscript;
                }
            }
            Token::StartTag { ref name, .. } if name == "script" => {
                if let Token::StartTag {
                    name, attributes, ..
                } = token
                {
                    self.insert_html_element(&name, &attributes);
                    self.tokenizer.set_state(State::ScriptData);
                    self.tokenizer.set_last_start_tag(&name);
                    self.original_mode = self.mode;
                    self.mode = InsertionMode::Text;
                }
            }
            Token::EndTag { ref name } if name == "head" => {
                self.open_elements.pop();
                self.mode = InsertionMode::AfterHead;
            }
            Token::EndTag { ref name } if matches!(name.as_str(), "body" | "html" | "br") => {
                self.open_elements.pop();
                self.mode = InsertionMode::AfterHead;
                self.process_token(token);
            }
            Token::StartTag { ref name, .. } if name == "template" => {
                if let Token::StartTag {
                    name, attributes, ..
                } = token
                {
                    self.insert_html_element(&name, &attributes);
                    self.push_formatting_marker();
                    self.frameset_ok = false;
                    self.mode = InsertionMode::InTemplate;
                    self.template_modes.push(InsertionMode::InTemplate);
                }
            }
            Token::EndTag { ref name } if name == "template" => {
                if self
                    .open_elements
                    .iter()
                    .any(|e| e.name == "template" && e.ns == Namespace::Html)
                {
                    self.generate_all_implied_end_tags();
                    while let Some(entry) = self.open_elements.pop() {
                        if entry.name == "template" && entry.ns == Namespace::Html {
                            break;
                        }
                    }
                    self.clear_formatting_to_marker();
                    self.template_modes.pop();
                    self.reset_insertion_mode();
                }
            }
            Token::StartTag { ref name, .. } if name == "head" => {
                // Parse error, ignore
            }
            Token::EndTag { .. } => {
                // Parse error, ignore
            }
            _ => {
                self.open_elements.pop();
                self.mode = InsertionMode::AfterHead;
                self.process_token(token);
            }
        }
    }

    fn handle_in_head_noscript(&mut self, token: Token) {
        match token {
            Token::Doctype { .. } => { /* ignore */ }
            Token::StartTag { ref name, .. } if name == "html" => {
                self.handle_in_body(token);
            }
            Token::EndTag { ref name } if name == "noscript" => {
                self.open_elements.pop();
                self.mode = InsertionMode::InHead;
            }
            Token::Character(c) if is_ascii_whitespace(c) => {
                self.handle_in_head(token);
            }
            Token::Comment(_) => {
                self.handle_in_head(token);
            }
            Token::StartTag { ref name, .. }
                if matches!(
                    name.as_str(),
                    "basefont" | "bgsound" | "link" | "meta" | "noframes" | "style"
                ) =>
            {
                self.handle_in_head(token);
            }
            Token::StartTag { ref name, .. } if matches!(name.as_str(), "head" | "noscript") => {
                // Parse error, ignore
            }
            Token::EndTag { ref name } if name != "br" => {
                // Parse error, ignore
            }
            _ => {
                self.open_elements.pop();
                self.mode = InsertionMode::InHead;
                self.process_token(token);
            }
        }
    }

    fn handle_after_head(&mut self, token: Token) {
        match token {
            Token::Character(c) if is_ascii_whitespace(c) => {
                self.insert_character(c);
            }
            Token::Comment(data) => self.insert_comment(&data),
            Token::Doctype { .. } => { /* ignore */ }
            Token::StartTag { ref name, .. } if name == "html" => {
                self.handle_in_body(token);
            }
            Token::StartTag { ref name, .. } if name == "body" => {
                if let Token::StartTag {
                    name, attributes, ..
                } = token
                {
                    self.insert_html_element(&name, &attributes);
                    self.frameset_ok = false;
                    self.mode = InsertionMode::InBody;
                }
            }
            Token::StartTag { ref name, .. } if name == "frameset" => {
                if let Token::StartTag {
                    name, attributes, ..
                } = token
                {
                    self.insert_html_element(&name, &attributes);
                    self.mode = InsertionMode::InFrameset;
                }
            }
            Token::StartTag { ref name, .. }
                if matches!(
                    name.as_str(),
                    "base"
                        | "basefont"
                        | "bgsound"
                        | "link"
                        | "meta"
                        | "noframes"
                        | "script"
                        | "style"
                        | "template"
                        | "title"
                ) =>
            {
                // Push head back, process in InHead, then remove head again
                if let Some(head) = self.head_pointer {
                    self.open_elements.push(StackEntry {
                        node_id: head,
                        name: "head".to_string(),
                        ns: Namespace::Html,
                        is_html_integration: false,
                    });
                }
                self.handle_in_head(token);
                // Remove head from stack if still there
                if let Some(pos) = self.open_elements.iter().position(|e| e.name == "head") {
                    self.open_elements.remove(pos);
                }
            }
            Token::EndTag { ref name } if name == "template" => {
                self.handle_in_head(token);
            }
            Token::StartTag { ref name, .. } if name == "head" => {
                // Parse error, ignore
            }
            Token::EndTag { ref name } if !matches!(name.as_str(), "body" | "html" | "br") => {
                // Parse error, ignore
            }
            _ => {
                self.insert_html_element("body", &[]);
                self.mode = InsertionMode::InBody;
                self.process_token(token);
            }
        }
    }

    #[allow(
        clippy::too_many_lines,
        clippy::cognitive_complexity,
        clippy::match_same_arms
    )]
    fn handle_in_body(&mut self, token: Token) {
        match token {
            Token::Character('\0') => { /* ignore */ }
            Token::Character(c) if is_ascii_whitespace(c) => {
                self.reconstruct_formatting();
                self.insert_character(c);
            }
            Token::Character(c) => {
                self.reconstruct_formatting();
                self.insert_character(c);
                self.frameset_ok = false;
            }
            Token::Comment(data) => self.insert_comment(&data),
            Token::Doctype { .. } => { /* ignore */ }
            Token::StartTag { ref name, .. } if name == "html" => {
                // Merge attributes onto the existing html element — but
                // ignore if there is a template on the stack.
                if !self.open_elements.iter().any(|e| e.name == "template") {
                    if let Token::StartTag { attributes, .. } = token {
                        if let Some(html_entry) = self.open_elements.first() {
                            let html_id = html_entry.node_id;
                            for attr in &attributes {
                                if self.doc.attribute(html_id, &attr.name).is_none() {
                                    if let NodeKind::Element {
                                        ref mut attributes, ..
                                    } = &mut self.doc.node_mut(html_id).kind
                                    {
                                        attributes.push(crate::tree::Attribute {
                                            name: attr.name.clone(),
                                            value: attr.value.clone(),
                                            prefix: None,
                                            namespace: None,
                                            raw_value: None,
                                        });
                                    }
                                }
                            }
                        }
                    }
                }
            }
            Token::StartTag { ref name, .. }
                if matches!(
                    name.as_str(),
                    "base"
                        | "basefont"
                        | "bgsound"
                        | "link"
                        | "meta"
                        | "noframes"
                        | "script"
                        | "style"
                        | "template"
                        | "title"
                ) =>
            {
                self.handle_in_head(token);
            }
            Token::EndTag { ref name } if name == "template" => {
                self.handle_in_head(token);
            }
            Token::StartTag { ref name, .. } if name == "body" => {
                // Merge attributes onto existing body — but ignore if there
                // is a template element on the stack of open elements.
                if let Token::StartTag { attributes, .. } = token {
                    if self.open_elements.len() >= 2
                        && self.open_elements[1].name == "body"
                        && !self.open_elements.iter().any(|e| e.name == "template")
                    {
                        let body_id = self.open_elements[1].node_id;
                        self.frameset_ok = false;
                        for attr in &attributes {
                            if self.doc.attribute(body_id, &attr.name).is_none() {
                                if let NodeKind::Element {
                                    ref mut attributes, ..
                                } = &mut self.doc.node_mut(body_id).kind
                                {
                                    attributes.push(crate::tree::Attribute {
                                        name: attr.name.clone(),
                                        value: attr.value.clone(),
                                        prefix: None,
                                        namespace: None,
                                        raw_value: None,
                                    });
                                }
                            }
                        }
                    }
                }
            }
            Token::StartTag { ref name, .. } if name == "frameset" => {
                // Ignore unless frameset_ok
                if self.frameset_ok {
                    if let Token::StartTag {
                        name, attributes, ..
                    } = token
                    {
                        // Remove body from stack if present
                        if self.open_elements.len() >= 2 && self.open_elements[1].name == "body" {
                            let body_id = self.open_elements[1].node_id;
                            self.doc.detach(body_id);
                            while self.open_elements.len() > 1 {
                                self.open_elements.pop();
                            }
                        }
                        self.insert_html_element(&name, &attributes);
                        self.mode = InsertionMode::InFrameset;
                    }
                }
            }
            Token::Eof => {
                if !self.template_modes.is_empty() {
                    self.handle_in_template(Token::Eof);
                }
                // Stop parsing
            }
            Token::EndTag { ref name } if name == "body" => {
                if self.element_in_scope("body") {
                    self.mode = InsertionMode::AfterBody;
                }
            }
            Token::EndTag { ref name } if name == "html" => {
                if self.element_in_scope("body") {
                    self.mode = InsertionMode::AfterBody;
                    self.process_token(token);
                }
            }
            Token::StartTag { ref name, .. }
                if matches!(
                    name.as_str(),
                    "address"
                        | "article"
                        | "aside"
                        | "blockquote"
                        | "center"
                        | "details"
                        | "dialog"
                        | "dir"
                        | "div"
                        | "dl"
                        | "fieldset"
                        | "figcaption"
                        | "figure"
                        | "footer"
                        | "header"
                        | "hgroup"
                        | "main"
                        | "menu"
                        | "nav"
                        | "ol"
                        | "p"
                        | "search"
                        | "section"
                        | "summary"
                        | "ul"
                ) =>
            {
                if self.element_in_button_scope("p") {
                    self.close_p_element();
                }
                if let Token::StartTag {
                    name, attributes, ..
                } = token
                {
                    self.insert_html_element(&name, &attributes);
                }
            }
            Token::StartTag { ref name, .. } if is_heading(name) => {
                if self.element_in_button_scope("p") {
                    self.close_p_element();
                }
                if is_heading(self.current_node_name()) {
                    self.open_elements.pop();
                }
                if let Token::StartTag {
                    name, attributes, ..
                } = token
                {
                    self.insert_html_element(&name, &attributes);
                }
            }
            Token::StartTag { ref name, .. } if matches!(name.as_str(), "pre" | "listing") => {
                if self.element_in_button_scope("p") {
                    self.close_p_element();
                }
                if let Token::StartTag {
                    name, attributes, ..
                } = token
                {
                    self.insert_html_element(&name, &attributes);
                    self.skip_next_lf = true;
                    self.frameset_ok = false;
                }
            }
            Token::StartTag { ref name, .. } if name == "form" => {
                if self.form_pointer.is_some()
                    && !self.open_elements.iter().any(|e| e.name == "template")
                {
                    // Parse error, ignore
                } else {
                    if self.element_in_button_scope("p") {
                        self.close_p_element();
                    }
                    if let Token::StartTag {
                        name, attributes, ..
                    } = token
                    {
                        let node_id = self.insert_html_element(&name, &attributes);
                        if !self.open_elements.iter().any(|e| e.name == "template") {
                            self.form_pointer = Some(node_id);
                        }
                    }
                }
            }
            Token::StartTag { ref name, .. } if name == "li" => {
                self.frameset_ok = false;
                // Close any open li in list item scope
                for i in (0..self.open_elements.len()).rev() {
                    let entry_name = self.open_elements[i].name.clone();
                    if entry_name == "li" {
                        self.generate_implied_end_tags(Some("li"));
                        while let Some(e) = self.open_elements.pop() {
                            if e.name == "li" {
                                break;
                            }
                        }
                        break;
                    }
                    if is_special_element_ns(&entry_name, self.open_elements[i].ns)
                        && !matches!(entry_name.as_str(), "address" | "div" | "p")
                    {
                        break;
                    }
                }
                if self.element_in_button_scope("p") {
                    self.close_p_element();
                }
                if let Token::StartTag {
                    name, attributes, ..
                } = token
                {
                    self.insert_html_element(&name, &attributes);
                }
            }
            Token::StartTag { ref name, .. } if matches!(name.as_str(), "dd" | "dt") => {
                self.frameset_ok = false;
                for i in (0..self.open_elements.len()).rev() {
                    let entry_name = self.open_elements[i].name.clone();
                    if matches!(entry_name.as_str(), "dd" | "dt") {
                        self.generate_implied_end_tags(Some(&entry_name));
                        while let Some(e) = self.open_elements.pop() {
                            if e.name == entry_name {
                                break;
                            }
                        }
                        break;
                    }
                    if is_special_element_ns(&entry_name, self.open_elements[i].ns)
                        && !matches!(entry_name.as_str(), "address" | "div" | "p")
                    {
                        break;
                    }
                }
                if self.element_in_button_scope("p") {
                    self.close_p_element();
                }
                if let Token::StartTag {
                    name, attributes, ..
                } = token
                {
                    self.insert_html_element(&name, &attributes);
                }
            }
            Token::StartTag { ref name, .. } if name == "plaintext" => {
                if self.element_in_button_scope("p") {
                    self.close_p_element();
                }
                if let Token::StartTag {
                    name, attributes, ..
                } = token
                {
                    self.insert_html_element(&name, &attributes);
                    self.tokenizer.set_state(State::Plaintext);
                }
            }
            Token::StartTag { ref name, .. } if name == "button" => {
                if self.element_in_scope("button") {
                    self.generate_implied_end_tags(None);
                    while let Some(e) = self.open_elements.pop() {
                        if e.name == "button" {
                            break;
                        }
                    }
                }
                self.reconstruct_formatting();
                if let Token::StartTag {
                    name, attributes, ..
                } = token
                {
                    self.insert_html_element(&name, &attributes);
                    self.frameset_ok = false;
                }
            }
            Token::EndTag { ref name }
                if matches!(
                    name.as_str(),
                    "address"
                        | "article"
                        | "aside"
                        | "blockquote"
                        | "button"
                        | "center"
                        | "details"
                        | "dialog"
                        | "dir"
                        | "div"
                        | "dl"
                        | "fieldset"
                        | "figcaption"
                        | "figure"
                        | "footer"
                        | "header"
                        | "hgroup"
                        | "listing"
                        | "main"
                        | "menu"
                        | "nav"
                        | "ol"
                        | "pre"
                        | "search"
                        | "section"
                        | "summary"
                        | "ul"
                ) =>
            {
                if let Token::EndTag { name } = token {
                    if self.element_in_scope(&name) {
                        self.generate_implied_end_tags(None);
                        while let Some(e) = self.open_elements.pop() {
                            if e.name == name {
                                break;
                            }
                        }
                    }
                }
            }
            Token::EndTag { ref name } if name == "form" => {
                if !self.open_elements.iter().any(|e| e.name == "template") {
                    let node = self.form_pointer.take();
                    if let Some(form_id) = node {
                        if self.element_in_scope("form") {
                            self.generate_implied_end_tags(None);
                            if let Some(pos) =
                                self.open_elements.iter().position(|e| e.node_id == form_id)
                            {
                                self.open_elements.remove(pos);
                            }
                        }
                    }
                } else if self.element_in_scope("form") {
                    self.generate_implied_end_tags(None);
                    while let Some(e) = self.open_elements.pop() {
                        if e.name == "form" {
                            break;
                        }
                    }
                }
            }
            Token::EndTag { ref name } if name == "p" => {
                if !self.element_in_button_scope("p") {
                    self.insert_html_element("p", &[]);
                }
                self.close_p_element();
            }
            Token::EndTag { ref name } if name == "li" => {
                if self.element_in_list_item_scope("li") {
                    self.generate_implied_end_tags(Some("li"));
                    while let Some(e) = self.open_elements.pop() {
                        if e.name == "li" {
                            break;
                        }
                    }
                }
            }
            Token::EndTag { ref name } if matches!(name.as_str(), "dd" | "dt") => {
                if let Token::EndTag { name } = token {
                    if self.element_in_scope(&name) {
                        self.generate_implied_end_tags(Some(&name));
                        while let Some(e) = self.open_elements.pop() {
                            if e.name == name {
                                break;
                            }
                        }
                    }
                }
            }
            Token::EndTag { ref name } if is_heading(name) => {
                if self.element_in_scope("h1")
                    || self.element_in_scope("h2")
                    || self.element_in_scope("h3")
                    || self.element_in_scope("h4")
                    || self.element_in_scope("h5")
                    || self.element_in_scope("h6")
                {
                    self.generate_implied_end_tags(None);
                    while let Some(e) = self.open_elements.pop() {
                        if is_heading(&e.name) {
                            break;
                        }
                    }
                }
            }
            Token::StartTag { ref name, .. } if name == "a" => {
                // Check if there's already an 'a' between the end of the
                // formatting list and the last marker (per spec §13.2.6.4.7).
                let existing_a = {
                    let mut found = None;
                    for (i, entry) in self.active_formatting.iter().enumerate().rev() {
                        match entry {
                            FormatEntry::Marker => break,
                            FormatEntry::Element { name, .. } if name == "a" => {
                                found = Some(i);
                                break;
                            }
                            FormatEntry::Element { .. } => {}
                        }
                    }
                    found
                };
                if existing_a.is_some() {
                    self.adoption_agency("a");
                    // Remove from formatting list if still there (only
                    // between end and last marker, matching the search above).
                    let mut remove_pos = None;
                    for (i, entry) in self.active_formatting.iter().enumerate().rev() {
                        match entry {
                            FormatEntry::Marker => break,
                            FormatEntry::Element { name, .. } if name == "a" => {
                                remove_pos = Some(i);
                                break;
                            }
                            FormatEntry::Element { .. } => {}
                        }
                    }
                    if let Some(pos) = remove_pos {
                        let entry = self.active_formatting.remove(pos);
                        if let FormatEntry::Element { node_id, .. } = entry {
                            if let Some(stack_pos) =
                                self.open_elements.iter().position(|e| e.node_id == node_id)
                            {
                                self.open_elements.remove(stack_pos);
                            }
                        }
                    }
                }
                self.reconstruct_formatting();
                if let Token::StartTag {
                    name, attributes, ..
                } = token
                {
                    let node_id = self.insert_html_element(&name, &attributes);
                    self.push_formatting(node_id, &name, &attributes);
                }
            }
            Token::StartTag { ref name, .. }
                if matches!(
                    name.as_str(),
                    "b" | "big"
                        | "code"
                        | "em"
                        | "font"
                        | "i"
                        | "s"
                        | "small"
                        | "strike"
                        | "strong"
                        | "tt"
                        | "u"
                ) =>
            {
                self.reconstruct_formatting();
                if let Token::StartTag {
                    name, attributes, ..
                } = token
                {
                    let node_id = self.insert_html_element(&name, &attributes);
                    self.push_formatting(node_id, &name, &attributes);
                }
            }
            Token::StartTag { ref name, .. } if name == "nobr" => {
                self.reconstruct_formatting();
                if self.element_in_scope("nobr") {
                    self.adoption_agency("nobr");
                    self.reconstruct_formatting();
                }
                if let Token::StartTag {
                    name, attributes, ..
                } = token
                {
                    let node_id = self.insert_html_element(&name, &attributes);
                    self.push_formatting(node_id, &name, &attributes);
                }
            }
            Token::EndTag { ref name } if is_formatting_element(name) => {
                if let Token::EndTag { name } = token {
                    self.adoption_agency(&name);
                }
            }
            Token::StartTag { ref name, .. }
                if matches!(name.as_str(), "applet" | "marquee" | "object") =>
            {
                self.reconstruct_formatting();
                if let Token::StartTag {
                    name, attributes, ..
                } = token
                {
                    self.insert_html_element(&name, &attributes);
                    self.push_formatting_marker();
                    self.frameset_ok = false;
                }
            }
            Token::EndTag { ref name }
                if matches!(name.as_str(), "applet" | "marquee" | "object") =>
            {
                if let Token::EndTag { name } = token {
                    if self.element_in_scope(&name) {
                        self.generate_implied_end_tags(None);
                        while let Some(e) = self.open_elements.pop() {
                            if e.name == name {
                                break;
                            }
                        }
                        self.clear_formatting_to_marker();
                    }
                }
            }
            Token::StartTag { ref name, .. } if name == "table" => {
                if self.quirks_mode != QuirksMode::Quirks && self.element_in_button_scope("p") {
                    self.close_p_element();
                }
                if let Token::StartTag {
                    name, attributes, ..
                } = token
                {
                    self.insert_html_element(&name, &attributes);
                    self.frameset_ok = false;
                    self.mode = InsertionMode::InTable;
                }
            }
            Token::EndTag { ref name } if name == "br" => {
                // Parse error — treat as start tag
                self.reconstruct_formatting();
                self.insert_html_element("br", &[]);
                self.open_elements.pop();
                self.frameset_ok = false;
            }
            Token::StartTag { ref name, .. }
                if matches!(
                    name.as_str(),
                    "area" | "br" | "embed" | "img" | "keygen" | "wbr"
                ) =>
            {
                self.reconstruct_formatting();
                if let Token::StartTag {
                    name, attributes, ..
                } = token
                {
                    self.insert_html_element(&name, &attributes);
                    self.open_elements.pop(); // void
                    self.frameset_ok = false;
                }
            }
            Token::StartTag { ref name, .. } if name == "input" => {
                self.reconstruct_formatting();
                if let Token::StartTag {
                    name, attributes, ..
                } = token
                {
                    let is_hidden = attributes
                        .iter()
                        .any(|a| a.name == "type" && a.value.eq_ignore_ascii_case("hidden"));
                    self.insert_html_element(&name, &attributes);
                    self.open_elements.pop(); // void
                    if !is_hidden {
                        self.frameset_ok = false;
                    }
                }
            }
            Token::StartTag { ref name, .. }
                if matches!(name.as_str(), "param" | "source" | "track") =>
            {
                if let Token::StartTag {
                    name, attributes, ..
                } = token
                {
                    self.insert_html_element(&name, &attributes);
                    self.open_elements.pop(); // void
                }
            }
            Token::StartTag { ref name, .. } if name == "hr" => {
                if self.element_in_button_scope("p") {
                    self.close_p_element();
                }
                if let Token::StartTag {
                    name, attributes, ..
                } = token
                {
                    self.insert_html_element(&name, &attributes);
                    self.open_elements.pop(); // void
                    self.frameset_ok = false;
                }
            }
            Token::StartTag { ref name, .. } if name == "image" => {
                // Parse error — change to "img"
                self.reconstruct_formatting();
                if let Token::StartTag { attributes, .. } = token {
                    self.insert_html_element("img", &attributes);
                    self.open_elements.pop();
                    self.frameset_ok = false;
                }
            }
            Token::StartTag { ref name, .. } if name == "textarea" => {
                if let Token::StartTag {
                    name, attributes, ..
                } = token
                {
                    self.insert_html_element(&name, &attributes);
                    self.skip_next_lf = true;
                    self.tokenizer.set_state(State::RcData);
                    self.tokenizer.set_last_start_tag(&name);
                    self.original_mode = self.mode;
                    self.frameset_ok = false;
                    self.mode = InsertionMode::Text;
                }
            }
            Token::StartTag { ref name, .. } if name == "xmp" => {
                if self.element_in_button_scope("p") {
                    self.close_p_element();
                }
                self.reconstruct_formatting();
                self.frameset_ok = false;
                if let Token::StartTag {
                    name, attributes, ..
                } = token
                {
                    self.parse_raw_text(&name, &attributes);
                }
            }
            Token::StartTag { ref name, .. } if name == "iframe" => {
                self.frameset_ok = false;
                if let Token::StartTag {
                    name, attributes, ..
                } = token
                {
                    self.parse_raw_text(&name, &attributes);
                }
            }
            Token::StartTag { ref name, .. } if name == "noembed" => {
                if let Token::StartTag {
                    name, attributes, ..
                } = token
                {
                    self.parse_raw_text(&name, &attributes);
                }
            }
            Token::StartTag { ref name, .. } if name == "noscript" && self.scripting => {
                if let Token::StartTag {
                    name, attributes, ..
                } = token
                {
                    self.parse_raw_text(&name, &attributes);
                }
            }
            Token::StartTag { ref name, .. } if name == "select" => {
                self.reconstruct_formatting();
                if let Token::StartTag {
                    name, attributes, ..
                } = token
                {
                    self.insert_html_element(&name, &attributes);
                    self.frameset_ok = false;
                    match self.mode {
                        InsertionMode::InTable
                        | InsertionMode::InCaption
                        | InsertionMode::InTableBody
                        | InsertionMode::InRow
                        | InsertionMode::InCell => {
                            self.mode = InsertionMode::InSelectInTable;
                        }
                        _ => {
                            self.mode = InsertionMode::InSelect;
                        }
                    }
                }
            }
            Token::StartTag { ref name, .. } if matches!(name.as_str(), "optgroup" | "option") => {
                if self.current_node_name() == "option" {
                    self.open_elements.pop();
                }
                self.reconstruct_formatting();
                if let Token::StartTag {
                    name, attributes, ..
                } = token
                {
                    self.insert_html_element(&name, &attributes);
                }
            }
            Token::StartTag { ref name, .. } if matches!(name.as_str(), "rb" | "rtc") => {
                if self.element_in_scope("ruby") {
                    self.generate_implied_end_tags(None);
                }
                if let Token::StartTag {
                    name, attributes, ..
                } = token
                {
                    self.insert_html_element(&name, &attributes);
                }
            }
            Token::StartTag { ref name, .. } if matches!(name.as_str(), "rp" | "rt") => {
                if self.element_in_scope("ruby") {
                    self.generate_implied_end_tags(Some("rtc"));
                }
                if let Token::StartTag {
                    name, attributes, ..
                } = token
                {
                    self.insert_html_element(&name, &attributes);
                }
            }
            Token::StartTag { ref name, .. } if name == "math" => {
                self.reconstruct_formatting();
                if let Token::StartTag {
                    name: _,
                    attributes,
                    self_closing,
                    ..
                } = token
                {
                    // Adjust MathML attributes and foreign attributes per spec.
                    let adjusted: Vec<tokenizer::Attribute> = attributes
                        .iter()
                        .map(|a| tokenizer::Attribute {
                            name: adjust_mathml_attributes(&a.name).to_string(),
                            value: a.value.clone(),
                        })
                        .collect();
                    self.insert_foreign_element("math", &adjusted, Namespace::MathMl);
                    if self_closing {
                        self.open_elements.pop();
                    }
                }
            }
            Token::StartTag { ref name, .. } if name == "svg" => {
                self.reconstruct_formatting();
                if let Token::StartTag {
                    name: _,
                    attributes,
                    self_closing,
                    ..
                } = token
                {
                    // Adjust SVG attributes and foreign attributes per spec.
                    let adjusted: Vec<tokenizer::Attribute> = attributes
                        .iter()
                        .map(|a| tokenizer::Attribute {
                            name: adjust_svg_attributes(&a.name).to_string(),
                            value: a.value.clone(),
                        })
                        .collect();
                    self.insert_foreign_element("svg", &adjusted, Namespace::Svg);
                    if self_closing {
                        self.open_elements.pop();
                    }
                }
            }
            Token::StartTag { ref name, .. }
                if matches!(
                    name.as_str(),
                    "caption"
                        | "col"
                        | "colgroup"
                        | "frame"
                        | "head"
                        | "tbody"
                        | "td"
                        | "tfoot"
                        | "th"
                        | "thead"
                        | "tr"
                ) =>
            {
                // Parse error, ignore
            }
            Token::StartTag {
                name, attributes, ..
            } => {
                // Any other start tag
                self.reconstruct_formatting();
                self.insert_html_element(&name, &attributes);
            }
            Token::EndTag { name } => {
                // Any other end tag
                self.handle_any_other_end_tag(&name);
            }
        }
    }

    fn handle_any_other_end_tag(&mut self, name: &str) {
        for i in (0..self.open_elements.len()).rev() {
            if self.open_elements[i].name == name && self.open_elements[i].ns == Namespace::Html {
                self.generate_implied_end_tags(Some(name));
                while self.open_elements.len() > i {
                    self.open_elements.pop();
                }
                return;
            }
            if is_special_element_ns(&self.open_elements[i].name, self.open_elements[i].ns) {
                return; // Parse error, ignore
            }
        }
    }

    #[allow(clippy::needless_pass_by_value)]
    fn handle_text(&mut self, token: Token) {
        match token {
            Token::Character(c) => {
                self.insert_character(c);
            }
            Token::Eof => {
                self.open_elements.pop();
                self.mode = self.original_mode;
                self.process_token(Token::Eof);
            }
            Token::EndTag { .. } => {
                self.open_elements.pop();
                self.mode = self.original_mode;
            }
            _ => {}
        }
    }

    #[allow(clippy::too_many_lines)]
    fn handle_in_table(&mut self, token: Token) {
        match token {
            Token::Character(_)
                if matches!(
                    self.current_node_name(),
                    "table" | "tbody" | "tfoot" | "thead" | "tr"
                ) =>
            {
                self.pending_table_chars.clear();
                self.original_mode = self.mode;
                self.mode = InsertionMode::InTableText;
                self.process_token(token);
            }
            Token::Comment(data) => self.insert_comment(&data),
            Token::Doctype { .. } => { /* ignore */ }
            Token::StartTag { ref name, .. } if name == "caption" => {
                self.clear_stack_back_to_table_context();
                self.push_formatting_marker();
                if let Token::StartTag {
                    name, attributes, ..
                } = token
                {
                    self.insert_html_element(&name, &attributes);
                    self.mode = InsertionMode::InCaption;
                }
            }
            Token::StartTag { ref name, .. } if name == "colgroup" => {
                self.clear_stack_back_to_table_context();
                if let Token::StartTag {
                    name, attributes, ..
                } = token
                {
                    self.insert_html_element(&name, &attributes);
                    self.mode = InsertionMode::InColumnGroup;
                }
            }
            Token::StartTag { ref name, .. } if name == "col" => {
                self.clear_stack_back_to_table_context();
                self.insert_html_element("colgroup", &[]);
                self.mode = InsertionMode::InColumnGroup;
                self.process_token(token);
            }
            Token::StartTag { ref name, .. }
                if matches!(name.as_str(), "tbody" | "tfoot" | "thead") =>
            {
                self.clear_stack_back_to_table_context();
                if let Token::StartTag {
                    name, attributes, ..
                } = token
                {
                    self.insert_html_element(&name, &attributes);
                    self.mode = InsertionMode::InTableBody;
                }
            }
            Token::StartTag { ref name, .. } if matches!(name.as_str(), "td" | "th" | "tr") => {
                self.clear_stack_back_to_table_context();
                self.insert_html_element("tbody", &[]);
                self.mode = InsertionMode::InTableBody;
                self.process_token(token);
            }
            Token::StartTag { ref name, .. } if name == "table" => {
                if self.element_in_table_scope("table") {
                    while let Some(e) = self.open_elements.pop() {
                        if e.name == "table" {
                            break;
                        }
                    }
                    self.reset_insertion_mode();
                    self.process_token(token);
                }
            }
            Token::EndTag { ref name } if name == "table" => {
                if self.element_in_table_scope("table") {
                    while let Some(e) = self.open_elements.pop() {
                        if e.name == "table" {
                            break;
                        }
                    }
                    self.reset_insertion_mode();
                }
            }
            Token::EndTag { ref name }
                if matches!(
                    name.as_str(),
                    "body"
                        | "caption"
                        | "col"
                        | "colgroup"
                        | "html"
                        | "tbody"
                        | "td"
                        | "tfoot"
                        | "th"
                        | "thead"
                        | "tr"
                ) =>
            {
                // Parse error, ignore
            }
            Token::StartTag { ref name, .. }
                if matches!(name.as_str(), "style" | "script" | "template") =>
            {
                self.handle_in_head(token);
            }
            Token::EndTag { ref name } if name == "template" => {
                self.handle_in_head(token);
            }
            Token::StartTag { ref name, .. } if name == "input" => {
                if let Token::StartTag { ref attributes, .. } = token {
                    let is_hidden = attributes
                        .iter()
                        .any(|a| a.name == "type" && a.value.eq_ignore_ascii_case("hidden"));
                    if is_hidden {
                        if let Token::StartTag {
                            name, attributes, ..
                        } = token
                        {
                            self.insert_html_element(&name, &attributes);
                            self.open_elements.pop();
                        }
                    } else {
                        self.foster_parenting = true;
                        self.handle_in_body(token);
                        self.foster_parenting = false;
                    }
                }
            }
            Token::StartTag { ref name, .. } if name == "form" => {
                if self.form_pointer.is_none()
                    && !self.open_elements.iter().any(|e| e.name == "template")
                {
                    if let Token::StartTag {
                        name, attributes, ..
                    } = token
                    {
                        let node_id = self.insert_html_element(&name, &attributes);
                        self.form_pointer = Some(node_id);
                        self.open_elements.pop();
                    }
                }
            }
            Token::Eof => {
                self.handle_in_body(token);
            }
            _ => {
                // Foster parenting
                self.foster_parenting = true;
                self.handle_in_body(token);
                self.foster_parenting = false;
            }
        }
    }

    fn handle_in_table_text(&mut self, token: Token) {
        match token {
            Token::Character('\0') => { /* ignore */ }
            Token::Character(c) => {
                self.pending_table_chars.push(c);
            }
            _ => {
                let chars: Vec<char> = std::mem::take(&mut self.pending_table_chars);
                let has_non_ws = chars.iter().any(|c| !is_ascii_whitespace(*c));
                if has_non_ws {
                    // Foster parent each character
                    self.foster_parenting = true;
                    for c in chars {
                        self.reconstruct_formatting();
                        self.insert_character(c);
                        if !is_ascii_whitespace(c) {
                            self.frameset_ok = false;
                        }
                    }
                    self.foster_parenting = false;
                } else {
                    for c in chars {
                        self.insert_character(c);
                    }
                }
                self.mode = self.original_mode;
                self.process_token(token);
            }
        }
    }

    fn handle_in_caption(&mut self, token: Token) {
        match token {
            Token::EndTag { ref name } if name == "caption" => {
                if self.element_in_table_scope("caption") {
                    self.generate_implied_end_tags(None);
                    while let Some(e) = self.open_elements.pop() {
                        if e.name == "caption" {
                            break;
                        }
                    }
                    self.clear_formatting_to_marker();
                    self.mode = InsertionMode::InTable;
                }
            }
            Token::StartTag { ref name, .. }
                if matches!(
                    name.as_str(),
                    "caption"
                        | "col"
                        | "colgroup"
                        | "tbody"
                        | "td"
                        | "tfoot"
                        | "th"
                        | "thead"
                        | "tr"
                ) =>
            {
                if self.element_in_table_scope("caption") {
                    self.generate_implied_end_tags(None);
                    while let Some(e) = self.open_elements.pop() {
                        if e.name == "caption" {
                            break;
                        }
                    }
                    self.clear_formatting_to_marker();
                    self.mode = InsertionMode::InTable;
                    self.process_token(token);
                }
            }
            Token::EndTag { ref name } if name == "table" => {
                if self.element_in_table_scope("caption") {
                    self.generate_implied_end_tags(None);
                    while let Some(e) = self.open_elements.pop() {
                        if e.name == "caption" {
                            break;
                        }
                    }
                    self.clear_formatting_to_marker();
                    self.mode = InsertionMode::InTable;
                    self.process_token(token);
                }
            }
            Token::EndTag { ref name }
                if matches!(
                    name.as_str(),
                    "body"
                        | "col"
                        | "colgroup"
                        | "html"
                        | "tbody"
                        | "td"
                        | "tfoot"
                        | "th"
                        | "thead"
                        | "tr"
                ) =>
            {
                // ignore
            }
            _ => {
                self.handle_in_body(token);
            }
        }
    }

    fn handle_in_column_group(&mut self, token: Token) {
        match token {
            Token::Character(c) if is_ascii_whitespace(c) => {
                self.insert_character(c);
            }
            Token::Comment(data) => self.insert_comment(&data),
            Token::Doctype { .. } => { /* ignore */ }
            Token::StartTag { ref name, .. } if name == "html" => {
                self.handle_in_body(token);
            }
            Token::StartTag { ref name, .. } if name == "col" => {
                if let Token::StartTag {
                    name, attributes, ..
                } = token
                {
                    self.insert_html_element(&name, &attributes);
                    self.open_elements.pop(); // void
                }
            }
            Token::EndTag { ref name } if name == "colgroup" => {
                if self.current_node_name() == "colgroup" {
                    self.open_elements.pop();
                    self.mode = InsertionMode::InTable;
                }
                // else: parse error, ignore
            }
            Token::EndTag { ref name } if name == "col" => {
                // parse error, ignore
            }
            Token::StartTag { ref name, .. } if name == "template" => {
                self.handle_in_head(token);
            }
            Token::EndTag { ref name } if name == "template" => {
                self.handle_in_head(token);
            }
            Token::Eof => {
                self.handle_in_body(token);
            }
            _ => {
                if self.current_node_name() == "colgroup" {
                    self.open_elements.pop();
                    self.mode = InsertionMode::InTable;
                    self.process_token(token);
                }
            }
        }
    }

    fn handle_in_table_body(&mut self, token: Token) {
        match token {
            Token::StartTag { ref name, .. } if name == "tr" => {
                self.clear_stack_back_to_table_body_context();
                if let Token::StartTag {
                    name, attributes, ..
                } = token
                {
                    self.insert_html_element(&name, &attributes);
                    self.mode = InsertionMode::InRow;
                }
            }
            Token::StartTag { ref name, .. } if matches!(name.as_str(), "th" | "td") => {
                self.clear_stack_back_to_table_body_context();
                self.insert_html_element("tr", &[]);
                self.mode = InsertionMode::InRow;
                self.process_token(token);
            }
            Token::EndTag { ref name } if matches!(name.as_str(), "tbody" | "tfoot" | "thead") => {
                if let Token::EndTag { name } = token {
                    if self.element_in_table_scope(&name) {
                        self.clear_stack_back_to_table_body_context();
                        self.open_elements.pop();
                        self.mode = InsertionMode::InTable;
                    }
                }
            }
            Token::StartTag { ref name, .. }
                if matches!(
                    name.as_str(),
                    "caption" | "col" | "colgroup" | "tbody" | "tfoot" | "thead"
                ) =>
            {
                if self.element_in_table_scope("tbody")
                    || self.element_in_table_scope("thead")
                    || self.element_in_table_scope("tfoot")
                {
                    self.clear_stack_back_to_table_body_context();
                    self.open_elements.pop();
                    self.mode = InsertionMode::InTable;
                    self.process_token(token);
                }
            }
            Token::EndTag { ref name } if name == "table" => {
                if self.element_in_table_scope("tbody")
                    || self.element_in_table_scope("thead")
                    || self.element_in_table_scope("tfoot")
                {
                    self.clear_stack_back_to_table_body_context();
                    self.open_elements.pop();
                    self.mode = InsertionMode::InTable;
                    self.process_token(token);
                }
            }
            Token::EndTag { ref name }
                if matches!(
                    name.as_str(),
                    "body" | "caption" | "col" | "colgroup" | "html" | "td" | "th" | "tr"
                ) =>
            {
                // ignore
            }
            _ => {
                self.handle_in_table(token);
            }
        }
    }

    fn handle_in_row(&mut self, token: Token) {
        match token {
            Token::StartTag { ref name, .. } if matches!(name.as_str(), "th" | "td") => {
                self.clear_stack_back_to_table_row_context();
                if let Token::StartTag {
                    name, attributes, ..
                } = token
                {
                    self.insert_html_element(&name, &attributes);
                    self.mode = InsertionMode::InCell;
                    self.push_formatting_marker();
                }
            }
            Token::EndTag { ref name } if name == "tr" => {
                if self.element_in_table_scope("tr") {
                    self.clear_stack_back_to_table_row_context();
                    self.open_elements.pop();
                    self.mode = InsertionMode::InTableBody;
                }
            }
            Token::StartTag { ref name, .. }
                if matches!(
                    name.as_str(),
                    "caption" | "col" | "colgroup" | "tbody" | "tfoot" | "thead" | "tr"
                ) =>
            {
                if self.element_in_table_scope("tr") {
                    self.clear_stack_back_to_table_row_context();
                    self.open_elements.pop();
                    self.mode = InsertionMode::InTableBody;
                    self.process_token(token);
                }
            }
            Token::EndTag { ref name } if name == "table" => {
                if self.element_in_table_scope("tr") {
                    self.clear_stack_back_to_table_row_context();
                    self.open_elements.pop();
                    self.mode = InsertionMode::InTableBody;
                    self.process_token(token);
                }
            }
            Token::EndTag { ref name } if matches!(name.as_str(), "tbody" | "tfoot" | "thead") => {
                if self.element_in_table_scope(name) && self.element_in_table_scope("tr") {
                    self.clear_stack_back_to_table_row_context();
                    self.open_elements.pop();
                    self.mode = InsertionMode::InTableBody;
                    self.process_token(token);
                }
            }
            Token::EndTag { ref name }
                if matches!(
                    name.as_str(),
                    "body" | "caption" | "col" | "colgroup" | "html" | "td" | "th"
                ) =>
            {
                // ignore
            }
            _ => {
                self.handle_in_table(token);
            }
        }
    }

    fn handle_in_cell(&mut self, token: Token) {
        match token {
            Token::EndTag { ref name } if matches!(name.as_str(), "td" | "th") => {
                if let Token::EndTag { name } = token {
                    if self.element_in_table_scope(&name) {
                        self.generate_implied_end_tags(None);
                        while let Some(e) = self.open_elements.pop() {
                            if e.name == name && e.ns == Namespace::Html {
                                break;
                            }
                        }
                        self.clear_formatting_to_marker();
                        self.mode = InsertionMode::InRow;
                    }
                }
            }
            Token::StartTag { ref name, .. }
                if matches!(
                    name.as_str(),
                    "caption"
                        | "col"
                        | "colgroup"
                        | "tbody"
                        | "td"
                        | "tfoot"
                        | "th"
                        | "thead"
                        | "tr"
                ) =>
            {
                if self.element_in_table_scope("td") || self.element_in_table_scope("th") {
                    self.close_cell();
                    self.process_token(token);
                }
            }
            Token::EndTag { ref name }
                if matches!(
                    name.as_str(),
                    "body" | "caption" | "col" | "colgroup" | "html"
                ) =>
            {
                // ignore
            }
            Token::EndTag { ref name }
                if matches!(name.as_str(), "table" | "tbody" | "tfoot" | "thead" | "tr") =>
            {
                if let Token::EndTag { ref name } = token {
                    if self.element_in_table_scope(name) {
                        self.close_cell();
                        self.process_token(token);
                    }
                }
            }
            _ => {
                self.handle_in_body(token);
            }
        }
    }

    fn close_cell(&mut self) {
        self.generate_implied_end_tags(None);
        while let Some(e) = self.open_elements.pop() {
            if matches!(e.name.as_str(), "td" | "th") {
                break;
            }
        }
        self.clear_formatting_to_marker();
        self.mode = InsertionMode::InRow;
    }

    #[allow(clippy::too_many_lines, clippy::match_same_arms)]
    fn handle_in_select(&mut self, token: Token) {
        match token {
            Token::Character('\0') => { /* ignore */ }
            Token::Character(c) => {
                self.reconstruct_formatting();
                self.insert_character(c);
            }
            Token::Comment(data) => self.insert_comment(&data),
            Token::Doctype { .. } => { /* ignore */ }
            Token::StartTag { ref name, .. } if name == "html" => {
                self.handle_in_body(token);
            }
            Token::StartTag { ref name, .. } if name == "option" => {
                if self.current_node_name() == "option" {
                    self.open_elements.pop();
                }
                self.reconstruct_formatting();
                if let Token::StartTag {
                    name, attributes, ..
                } = token
                {
                    self.insert_html_element(&name, &attributes);
                }
            }
            Token::StartTag { ref name, .. } if name == "optgroup" => {
                if self.current_node_name() == "option" {
                    self.open_elements.pop();
                }
                if self.current_node_name() == "optgroup" {
                    self.open_elements.pop();
                }
                if let Token::StartTag {
                    name, attributes, ..
                } = token
                {
                    self.insert_html_element(&name, &attributes);
                }
            }
            Token::StartTag { ref name, .. } if name == "hr" => {
                if self.current_node_name() == "option" {
                    self.open_elements.pop();
                }
                if self.current_node_name() == "optgroup" {
                    self.open_elements.pop();
                }
                if let Token::StartTag {
                    name, attributes, ..
                } = token
                {
                    self.insert_html_element(&name, &attributes);
                    self.open_elements.pop(); // void
                }
            }
            Token::EndTag { ref name } if name == "optgroup" => {
                if self.current_node_name() == "option"
                    && self.open_elements.len() >= 2
                    && self.open_elements[self.open_elements.len() - 2].name == "optgroup"
                {
                    self.open_elements.pop();
                }
                if self.current_node_name() == "optgroup" {
                    self.open_elements.pop();
                }
            }
            Token::EndTag { ref name } if name == "option" => {
                if self.current_node_name() == "option" {
                    self.open_elements.pop();
                }
            }
            Token::EndTag { ref name } if name == "select" => {
                if self.element_in_select_scope("select") {
                    while let Some(e) = self.open_elements.pop() {
                        if e.name == "select" {
                            break;
                        }
                    }
                    self.reset_insertion_mode();
                }
            }
            Token::StartTag { ref name, .. } if name == "select" => {
                // Parse error — act as end tag
                if self.element_in_select_scope("select") {
                    while let Some(e) = self.open_elements.pop() {
                        if e.name == "select" {
                            break;
                        }
                    }
                    self.reset_insertion_mode();
                }
            }
            Token::StartTag { ref name, .. } if matches!(name.as_str(), "input" | "textarea") => {
                // Close select and reprocess
                if self.element_in_select_scope("select") {
                    while let Some(e) = self.open_elements.pop() {
                        if e.name == "select" {
                            break;
                        }
                    }
                    self.reset_insertion_mode();
                    self.process_token(token);
                } else if self
                    .fragment_context
                    .as_ref()
                    .is_some_and(|(n, ns)| n == "select" && *ns == Namespace::Html)
                {
                    // Fragment case: context is select but it's not on the stack.
                    // Switch to InBody and reprocess.
                    self.mode = InsertionMode::InBody;
                    self.process_token(token);
                }
            }
            Token::StartTag { ref name, .. } if matches!(name.as_str(), "script" | "template") => {
                self.handle_in_head(token);
            }
            Token::EndTag { ref name } if name == "template" => {
                self.handle_in_head(token);
            }
            // New select content model: allow certain elements inside <select>.
            Token::StartTag { ref name, .. }
                if matches!(
                    name.as_str(),
                    "div" | "button" | "datalist" | "selectedcontent"
                ) =>
            {
                if let Token::StartTag {
                    name, attributes, ..
                } = token
                {
                    self.insert_html_element(&name, &attributes);
                }
            }
            Token::EndTag { ref name }
                if matches!(
                    name.as_str(),
                    "div" | "button" | "datalist" | "selectedcontent"
                ) =>
            {
                if self.element_in_scope(name) {
                    self.generate_implied_end_tags(Some(name));
                    while let Some(e) = self.open_elements.pop() {
                        if e.name == *name {
                            break;
                        }
                    }
                }
            }
            Token::StartTag { ref name, .. } if name == "svg" => {
                self.reconstruct_formatting();
                if let Token::StartTag {
                    name: _,
                    attributes,
                    self_closing,
                    ..
                } = token
                {
                    let adjusted: Vec<tokenizer::Attribute> = attributes
                        .iter()
                        .map(|a| tokenizer::Attribute {
                            name: adjust_svg_attributes(&a.name).to_string(),
                            value: a.value.clone(),
                        })
                        .collect();
                    self.insert_foreign_element("svg", &adjusted, Namespace::Svg);
                    if self_closing {
                        self.open_elements.pop();
                    }
                }
            }
            Token::StartTag { ref name, .. } if name == "math" => {
                self.reconstruct_formatting();
                if let Token::StartTag {
                    name: _,
                    attributes,
                    self_closing,
                    ..
                } = token
                {
                    let adjusted: Vec<tokenizer::Attribute> = attributes
                        .iter()
                        .map(|a| tokenizer::Attribute {
                            name: adjust_mathml_attributes(&a.name).to_string(),
                            value: a.value.clone(),
                        })
                        .collect();
                    self.insert_foreign_element("math", &adjusted, Namespace::MathMl);
                    if self_closing {
                        self.open_elements.pop();
                    }
                }
            }
            // New select content model: allow most other start tags
            // by processing them via InBody rules.
            Token::StartTag { .. } => {
                self.handle_in_body(token);
            }
            // End tags for elements opened inside select via InBody.
            Token::EndTag { ref name }
                if self
                    .open_elements
                    .iter()
                    .rev()
                    .take_while(|e| e.name != "select")
                    .any(|e| e.name == *name && e.ns == Namespace::Html) =>
            {
                self.handle_in_body(token);
            }
            Token::Eof => {
                self.handle_in_body(token);
            }
            Token::EndTag { .. } => { /* ignore */ }
        }
    }

    fn handle_in_select_in_table(&mut self, token: Token) {
        match token {
            Token::StartTag { ref name, .. }
                if matches!(
                    name.as_str(),
                    "caption" | "table" | "tbody" | "tfoot" | "thead" | "tr" | "td" | "th"
                ) =>
            {
                while let Some(e) = self.open_elements.pop() {
                    if e.name == "select" {
                        break;
                    }
                }
                self.reset_insertion_mode();
                self.process_token(token);
            }
            Token::EndTag { ref name }
                if matches!(
                    name.as_str(),
                    "caption" | "table" | "tbody" | "tfoot" | "thead" | "tr" | "td" | "th"
                ) =>
            {
                if let Token::EndTag { ref name } = token {
                    if self.element_in_table_scope(name) {
                        while let Some(e) = self.open_elements.pop() {
                            if e.name == "select" {
                                break;
                            }
                        }
                        self.reset_insertion_mode();
                        self.process_token(token);
                    }
                }
            }
            _ => {
                self.handle_in_select(token);
            }
        }
    }

    #[allow(clippy::match_same_arms)]
    fn handle_in_template(&mut self, token: Token) {
        match token {
            Token::Character(_) | Token::Comment(_) | Token::Doctype { .. } => {
                self.handle_in_body(token);
            }
            Token::StartTag { ref name, .. }
                if matches!(
                    name.as_str(),
                    "base"
                        | "basefont"
                        | "bgsound"
                        | "link"
                        | "meta"
                        | "noframes"
                        | "script"
                        | "style"
                        | "template"
                        | "title"
                ) =>
            {
                self.handle_in_head(token);
            }
            Token::EndTag { ref name } if name == "template" => {
                self.handle_in_head(token);
            }
            Token::StartTag { ref name, .. }
                if matches!(
                    name.as_str(),
                    "caption" | "colgroup" | "tbody" | "tfoot" | "thead"
                ) =>
            {
                self.template_modes.pop();
                self.template_modes.push(InsertionMode::InTable);
                self.mode = InsertionMode::InTable;
                self.process_token(token);
            }
            Token::StartTag { ref name, .. } if name == "col" => {
                self.template_modes.pop();
                self.template_modes.push(InsertionMode::InColumnGroup);
                self.mode = InsertionMode::InColumnGroup;
                self.process_token(token);
            }
            Token::StartTag { ref name, .. } if name == "tr" => {
                self.template_modes.pop();
                self.template_modes.push(InsertionMode::InTableBody);
                self.mode = InsertionMode::InTableBody;
                self.process_token(token);
            }
            Token::StartTag { ref name, .. } if matches!(name.as_str(), "td" | "th") => {
                self.template_modes.pop();
                self.template_modes.push(InsertionMode::InRow);
                self.mode = InsertionMode::InRow;
                self.process_token(token);
            }
            Token::Eof => {
                if self
                    .open_elements
                    .iter()
                    .any(|e| e.name == "template" && e.ns == Namespace::Html)
                {
                    self.generate_all_implied_end_tags();
                    while let Some(e) = self.open_elements.pop() {
                        if e.name == "template" && e.ns == Namespace::Html {
                            break;
                        }
                    }
                    self.clear_formatting_to_marker();
                    self.template_modes.pop();
                    self.reset_insertion_mode();
                    self.process_token(Token::Eof);
                }
                // else: stop parsing
            }
            Token::StartTag { .. } => {
                self.template_modes.pop();
                self.template_modes.push(InsertionMode::InBody);
                self.mode = InsertionMode::InBody;
                self.process_token(token);
            }
            Token::EndTag { .. } => {
                // ignore
            }
        }
    }

    #[allow(clippy::match_same_arms)]
    fn handle_after_body(&mut self, token: Token) {
        match token {
            Token::Character(c) if is_ascii_whitespace(c) => {
                self.handle_in_body(token);
            }
            Token::Comment(data) => {
                // Append to the html element (first in stack)
                if let Some(html_entry) = self.open_elements.first() {
                    let html_id = html_entry.node_id;
                    let comment_id = self.doc.create_node(NodeKind::Comment { content: data });
                    self.doc.append_child(html_id, comment_id);
                }
            }
            Token::Doctype { .. } => { /* ignore */ }
            Token::StartTag { ref name, .. } if name == "html" => {
                self.handle_in_body(token);
            }
            Token::EndTag { ref name } if name == "html" => {
                if self.fragment_context.is_some() {
                    // Fragment case: ignore the token (parse error).
                } else {
                    self.mode = InsertionMode::AfterAfterBody;
                }
            }
            Token::Eof => {
                // Stop parsing
            }
            _ => {
                self.mode = InsertionMode::InBody;
                self.process_token(token);
            }
        }
    }

    #[allow(clippy::match_same_arms)]
    fn handle_in_frameset(&mut self, token: Token) {
        match token {
            Token::Character(c) if is_ascii_whitespace(c) => {
                self.insert_character(c);
            }
            Token::Comment(data) => self.insert_comment(&data),
            Token::Doctype { .. } => { /* ignore */ }
            Token::StartTag { ref name, .. } if name == "html" => {
                self.handle_in_body(token);
            }
            Token::StartTag { ref name, .. } if name == "frameset" => {
                if let Token::StartTag {
                    name, attributes, ..
                } = token
                {
                    self.insert_html_element(&name, &attributes);
                }
            }
            Token::EndTag { ref name }
                if name == "frameset" && self.current_node_name() != "html" =>
            {
                self.open_elements.pop();
                if self.current_node_name() != "frameset" {
                    self.mode = InsertionMode::AfterFrameset;
                }
            }
            Token::StartTag { ref name, .. } if name == "frame" => {
                if let Token::StartTag {
                    name, attributes, ..
                } = token
                {
                    self.insert_html_element(&name, &attributes);
                    self.open_elements.pop(); // void
                }
            }
            Token::StartTag { ref name, .. } if name == "noframes" => {
                self.handle_in_head(token);
            }
            Token::Eof => {
                // Stop parsing
            }
            _ => { /* ignore */ }
        }
    }

    #[allow(clippy::match_same_arms)]
    fn handle_after_frameset(&mut self, token: Token) {
        match token {
            Token::Character(c) if is_ascii_whitespace(c) => {
                self.insert_character(c);
            }
            Token::Comment(data) => self.insert_comment(&data),
            Token::Doctype { .. } => { /* ignore */ }
            Token::StartTag { ref name, .. } if name == "html" => {
                self.handle_in_body(token);
            }
            Token::EndTag { ref name } if name == "html" => {
                self.mode = InsertionMode::AfterAfterFrameset;
            }
            Token::StartTag { ref name, .. } if name == "noframes" => {
                self.handle_in_head(token);
            }
            Token::Eof => {
                // Stop parsing
            }
            _ => { /* ignore */ }
        }
    }

    #[allow(clippy::match_same_arms)]
    fn handle_after_after_body(&mut self, token: Token) {
        match token {
            Token::Comment(data) => {
                self.insert_comment_at_document(&data);
            }
            Token::Doctype { .. } | Token::Character(' ' | '\t' | '\n' | '\x0C' | '\r') => {
                self.handle_in_body(token);
            }
            Token::StartTag { ref name, .. } if name == "html" => {
                self.handle_in_body(token);
            }
            Token::Eof => {
                // Stop parsing
            }
            _ => {
                self.mode = InsertionMode::InBody;
                self.process_token(token);
            }
        }
    }

    #[allow(clippy::match_same_arms)]
    fn handle_after_after_frameset(&mut self, token: Token) {
        match token {
            Token::Comment(data) => {
                self.insert_comment_at_document(&data);
            }
            Token::Doctype { .. } | Token::Character(' ' | '\t' | '\n' | '\x0C' | '\r') => {
                self.handle_in_body(token);
            }
            Token::StartTag { ref name, .. } if name == "html" => {
                self.handle_in_body(token);
            }
            Token::StartTag { ref name, .. } if name == "noframes" => {
                self.handle_in_head(token);
            }
            Token::Eof => {
                // Stop parsing
            }
            _ => { /* ignore */ }
        }
    }

    // -----------------------------------------------------------------------
    // Stack clearing helpers
    // -----------------------------------------------------------------------

    fn clear_stack_back_to_table_context(&mut self) {
        while !self.open_elements.is_empty() {
            if matches!(self.current_node_name(), "table" | "template" | "html") {
                break;
            }
            self.open_elements.pop();
        }
    }

    fn clear_stack_back_to_table_body_context(&mut self) {
        while !self.open_elements.is_empty() {
            if matches!(
                self.current_node_name(),
                "tbody" | "tfoot" | "thead" | "template" | "html"
            ) {
                break;
            }
            self.open_elements.pop();
        }
    }

    fn clear_stack_back_to_table_row_context(&mut self) {
        while !self.open_elements.is_empty() {
            if matches!(self.current_node_name(), "tr" | "template" | "html") {
                break;
            }
            self.open_elements.pop();
        }
    }

    fn reset_insertion_mode(&mut self) {
        for i in (0..self.open_elements.len()).rev() {
            let last = i == 0;
            // Per WHATWG §13.2.4.1: when last is true and this is fragment
            // parsing, use the context element instead of the stack element.
            let name = if last {
                if let Some((ref ctx_name, _)) = self.fragment_context {
                    ctx_name.clone()
                } else {
                    self.open_elements[i].name.clone()
                }
            } else {
                self.open_elements[i].name.clone()
            };
            match name.as_str() {
                "select" => {
                    if !last {
                        // Walk up to find if we're in a table
                        for j in (0..i).rev() {
                            match self.open_elements[j].name.as_str() {
                                "template" => break,
                                "table" => {
                                    self.mode = InsertionMode::InSelectInTable;
                                    return;
                                }
                                _ => {}
                            }
                        }
                    }
                    self.mode = InsertionMode::InSelect;
                    return;
                }
                "td" | "th" if !last => {
                    self.mode = InsertionMode::InCell;
                    return;
                }
                "tr" => {
                    self.mode = InsertionMode::InRow;
                    return;
                }
                "tbody" | "thead" | "tfoot" => {
                    self.mode = InsertionMode::InTableBody;
                    return;
                }
                "caption" => {
                    self.mode = InsertionMode::InCaption;
                    return;
                }
                "colgroup" => {
                    self.mode = InsertionMode::InColumnGroup;
                    return;
                }
                "table" => {
                    self.mode = InsertionMode::InTable;
                    return;
                }
                "template" => {
                    self.mode = self
                        .template_modes
                        .last()
                        .copied()
                        .unwrap_or(InsertionMode::InBody);
                    return;
                }
                "head" if !last => {
                    self.mode = InsertionMode::InHead;
                    return;
                }
                "body" => {
                    self.mode = InsertionMode::InBody;
                    return;
                }
                "frameset" => {
                    self.mode = InsertionMode::InFrameset;
                    return;
                }
                "html" => {
                    if self.head_pointer.is_none() {
                        self.mode = InsertionMode::BeforeHead;
                    } else {
                        self.mode = InsertionMode::AfterHead;
                    }
                    return;
                }
                _ => {}
            }
            if last {
                self.mode = InsertionMode::InBody;
                return;
            }
        }
        self.mode = InsertionMode::InBody;
    }
}

// ---------------------------------------------------------------------------
// Utility
// ---------------------------------------------------------------------------

fn is_ascii_whitespace(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\x0C' | '\r')
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    /// Helper: parse HTML5, return the Document.
    fn parse(input: &str) -> Document {
        parse_html5(input).unwrap()
    }

    /// Walk children and collect their node names (or "#text" / "#comment").
    fn child_names(doc: &Document, id: NodeId) -> Vec<String> {
        doc.children(id)
            .map(|c| match &doc.node(c).kind {
                NodeKind::Element { name, .. } => name.clone(),
                NodeKind::Text { .. } => "#text".to_string(),
                NodeKind::Comment { .. } => "#comment".to_string(),
                NodeKind::DocumentType { .. } => "#doctype".to_string(),
                NodeKind::Document => "#document".to_string(),
                _ => "#other".to_string(),
            })
            .collect()
    }

    #[test]
    fn test_simple_text() {
        let doc = parse("hello");
        let html = doc.root_element().unwrap();
        assert_eq!(doc.node_name(html), Some("html"));
        let children = child_names(&doc, html);
        assert_eq!(children, vec!["head", "body"]);
        let body = doc.children(html).nth(1).unwrap();
        assert_eq!(doc.text_content(body), "hello");
    }

    #[test]
    fn test_basic_element() {
        let doc = parse("<p>hi</p>");
        let html = doc.root_element().unwrap();
        assert_eq!(doc.node_name(html), Some("html"));
        let body = doc.children(html).nth(1).unwrap();
        assert_eq!(doc.node_name(body), Some("body"));
        let p = doc.first_child(body).unwrap();
        assert_eq!(doc.node_name(p), Some("p"));
        assert_eq!(doc.text_content(p), "hi");
    }

    #[test]
    fn test_implied_tags() {
        let doc = parse("test");
        let html = doc.root_element().unwrap();
        assert_eq!(doc.node_name(html), Some("html"));
        let children = child_names(&doc, html);
        assert_eq!(children, vec!["head", "body"]);
    }

    #[test]
    fn test_nested_elements() {
        let doc = parse("<div><p>text</p></div>");
        let html = doc.root_element().unwrap();
        let body = doc.children(html).nth(1).unwrap();
        let div = doc.first_child(body).unwrap();
        assert_eq!(doc.node_name(div), Some("div"));
        let p = doc.first_child(div).unwrap();
        assert_eq!(doc.node_name(p), Some("p"));
        assert_eq!(doc.text_content(p), "text");
    }

    #[test]
    fn test_auto_closing_p() {
        let doc = parse("<p>one<p>two");
        let html = doc.root_element().unwrap();
        let body = doc.children(html).nth(1).unwrap();
        let children = child_names(&doc, body);
        assert_eq!(children, vec!["p", "p"]);
        let p1 = doc.first_child(body).unwrap();
        let p2 = doc.next_sibling(p1).unwrap();
        assert_eq!(doc.text_content(p1), "one");
        assert_eq!(doc.text_content(p2), "two");
    }

    #[test]
    fn test_formatting_elements() {
        let doc = parse("<b>bold</b>normal");
        let html = doc.root_element().unwrap();
        let body = doc.children(html).nth(1).unwrap();
        let b = doc.first_child(body).unwrap();
        assert_eq!(doc.node_name(b), Some("b"));
        assert_eq!(doc.text_content(b), "bold");
        let text = doc.next_sibling(b).unwrap();
        assert_eq!(doc.node_text(text), Some("normal"));
    }

    #[test]
    fn test_adoption_agency() {
        // Classic misnesting: <b><i>bi</b>i</i>
        // Expected: <b><i>bi</i></b><i>i</i>
        let doc = parse("<b><i>bi</b>i</i>");
        let html = doc.root_element().unwrap();
        let body = doc.children(html).nth(1).unwrap();
        let body_children = child_names(&doc, body);
        // Should have: b, i
        assert_eq!(body_children.len(), 2);
        let b_elem = doc.first_child(body).unwrap();
        assert_eq!(doc.node_name(b_elem), Some("b"));
        let i_in_b = doc.first_child(b_elem).unwrap();
        assert_eq!(doc.node_name(i_in_b), Some("i"));
        assert_eq!(doc.text_content(i_in_b), "bi");
        let i_after_b = doc.next_sibling(b_elem).unwrap();
        assert_eq!(doc.node_name(i_after_b), Some("i"));
        assert_eq!(doc.text_content(i_after_b), "i");
    }

    #[test]
    fn test_void_elements() {
        let doc = parse("<br><img><hr>");
        let html = doc.root_element().unwrap();
        let body = doc.children(html).nth(1).unwrap();
        let children = child_names(&doc, body);
        assert_eq!(children, vec!["br", "img", "hr"]);
        // Void elements should have no children
        let br = doc.first_child(body).unwrap();
        assert!(doc.first_child(br).is_none());
    }

    #[test]
    fn test_table_structure() {
        let doc = parse("<table><tr><td>cell</td></tr></table>");
        let html = doc.root_element().unwrap();
        let body = doc.children(html).nth(1).unwrap();
        let table = doc.first_child(body).unwrap();
        assert_eq!(doc.node_name(table), Some("table"));
        let tbody = doc.first_child(table).unwrap();
        assert_eq!(doc.node_name(tbody), Some("tbody"));
        let tr = doc.first_child(tbody).unwrap();
        assert_eq!(doc.node_name(tr), Some("tr"));
        let td = doc.first_child(tr).unwrap();
        assert_eq!(doc.node_name(td), Some("td"));
        assert_eq!(doc.text_content(td), "cell");
    }

    #[test]
    fn test_doctype() {
        let doc = parse("<!DOCTYPE html><html><body>hi</body></html>");
        let root = doc.root();
        // First child should be the doctype
        let first = doc.first_child(root).unwrap();
        assert!(matches!(
            doc.node(first).kind,
            NodeKind::DocumentType { .. }
        ));
        let html = doc.root_element().unwrap();
        assert_eq!(doc.node_name(html), Some("html"));
        let body = doc.children(html).nth(1).unwrap();
        assert_eq!(doc.text_content(body), "hi");
    }

    #[test]
    fn test_comment() {
        let doc = parse("<!-- comment --><p>text</p>");
        let html = doc.root_element().unwrap();
        let body = doc.children(html).nth(1).unwrap();
        let p = doc.first_child(body).unwrap();
        assert_eq!(doc.node_name(p), Some("p"));
        assert_eq!(doc.text_content(p), "text");
    }

    #[test]
    fn test_self_closing_svg() {
        let doc = parse("<svg><circle/></svg>");
        let html = doc.root_element().unwrap();
        let body = doc.children(html).nth(1).unwrap();
        let svg = doc.first_child(body).unwrap();
        assert_eq!(doc.node_name(svg), Some("svg"));
    }

    #[test]
    fn test_template() {
        let doc = parse("<template><p>content</p></template>");
        let html = doc.root_element().unwrap();
        let head = doc.first_child(html).unwrap();
        assert_eq!(doc.node_name(head), Some("head"));
        let template = doc.first_child(head).unwrap();
        assert_eq!(doc.node_name(template), Some("template"));
    }

    #[test]
    fn test_select() {
        let doc = parse("<select><option>a</option><option>b</option></select>");
        let html = doc.root_element().unwrap();
        let body = doc.children(html).nth(1).unwrap();
        let select = doc.first_child(body).unwrap();
        assert_eq!(doc.node_name(select), Some("select"));
        let children = child_names(&doc, select);
        assert_eq!(children, vec!["option", "option"]);
    }
}
