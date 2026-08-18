//! xmllint-compatible CLI tool for XML/HTML processing.
//!
//! Provides the most commonly used features of libxml2's `xmllint` command:
//! parsing, validation, `XPath` evaluation, canonical XML output, and more.

use std::fmt::Write as _;
use std::fs;
use std::io::{self, Read, Write};
use std::process::ExitCode;
use std::time::Instant;

use clap::Parser;

use xmloxide::html::parse_html_with_options;
use xmloxide::html5::parse_html5;
use xmloxide::parser::{self, ParseOptions};
use xmloxide::serial::c14n::{canonicalize, C14nOptions};
use xmloxide::serial::serialize;
use xmloxide::tree::{Document, NodeId, NodeKind};
use xmloxide::xpath;

// ---------------------------------------------------------------------------
// CLI argument definitions
// ---------------------------------------------------------------------------

/// xmllint -- parse, validate, and process XML/HTML files.
///
/// A Rust reimplementation of libxml2's xmllint, powered by xmloxide.
#[derive(Parser, Debug)]
#[command(name = "xmllint", version, about, long_about = None)]
#[allow(clippy::struct_excessive_bools)]
struct Cli {
    /// XML files to process (use `-` for stdin).
    #[arg(required = true)]
    files: Vec<String>,

    /// Print additional information during processing.
    #[arg(long)]
    verbose: bool,

    // -- Parsing options ---------------------------------------------------
    /// Parse input as HTML 4.01 instead of XML.
    #[arg(long)]
    html: bool,

    /// Parse input as HTML5 (WHATWG) instead of XML.
    #[arg(long)]
    html5: bool,

    /// Recover from parsing errors (produce partial tree).
    #[arg(long)]
    recover: bool,

    /// Remove ignorable blank (whitespace-only) text nodes.
    #[arg(long)]
    noblanks: bool,

    /// Do not output the result tree.
    #[arg(long)]
    noout: bool,

    /// Output in the given encoding (e.g., UTF-8, ISO-8859-1).
    #[arg(long, value_name = "ENCODING")]
    encode: Option<String>,

    // -- Validation options ------------------------------------------------
    /// Validate against the DTD declared in the document.
    #[arg(long)]
    valid: bool,

    /// Validate against an external DTD file.
    #[arg(long, value_name = "FILE")]
    dtdvalid: Option<String>,

    /// Validate against a RelaxNG schema file.
    #[allow(clippy::doc_markdown)]
    #[arg(long, value_name = "FILE")]
    relaxng: Option<String>,

    /// Validate against an XML Schema (XSD) file.
    #[arg(long, value_name = "FILE")]
    schema: Option<String>,

    /// Validate against an ISO Schematron schema file.
    #[arg(long, value_name = "FILE")]
    schematron: Option<String>,

    // -- XPath -------------------------------------------------------------
    /// Evaluate an XPath expression and print the result.
    #[allow(clippy::doc_markdown)]
    #[arg(long, value_name = "EXPR")]
    xpath: Option<String>,

    // -- Output options ----------------------------------------------------
    /// Pretty-print (indent) the output.
    #[arg(long)]
    format: bool,

    /// Canonical XML (C14N 1.0) output.
    #[arg(long)]
    c14n: bool,

    /// Exclusive Canonical XML output.
    #[arg(long = "exc-c14n")]
    exc_c14n: bool,

    /// Save output to a file instead of stdout.
    #[arg(long, value_name = "FILE")]
    output: Option<String>,

    // -- Debug options -----------------------------------------------------
    /// Print a debug representation of the document tree.
    #[arg(long)]
    debug: bool,

    /// Print timing information for parsing and processing.
    #[arg(long)]
    timing: bool,
}

// ---------------------------------------------------------------------------
// Exit codes (matching libxml2 xmllint conventions)
// ---------------------------------------------------------------------------

const EXIT_SUCCESS: u8 = 0;
const EXIT_PARSE_ERROR: u8 = 1;
const EXIT_VALIDATION_ERROR: u8 = 3;

// ---------------------------------------------------------------------------
// Main entry point
// ---------------------------------------------------------------------------

fn main() -> ExitCode {
    let cli = Cli::parse();
    let mut worst_exit: u8 = EXIT_SUCCESS;

    for file in &cli.files {
        let exit = process_file(&cli, file);
        if exit > worst_exit {
            worst_exit = exit;
        }
    }

    ExitCode::from(worst_exit)
}

/// Processes a single input file and returns an exit code.
fn process_file(cli: &Cli, filename: &str) -> u8 {
    // -- Read input --------------------------------------------------------
    let start_read = Instant::now();

    let input = match read_input(filename) {
        Ok(data) => data,
        Err(e) => {
            eprintln!("{filename}: failed to read: {e}");
            return EXIT_PARSE_ERROR;
        }
    };

    if cli.timing {
        let elapsed = start_read.elapsed();
        eprintln!("Reading file {filename} took {elapsed:?}");
    }

    // -- Parse -------------------------------------------------------------
    let start_parse = Instant::now();

    let doc = if cli.html5 {
        parse_as_html5(&input)
    } else if cli.html {
        parse_as_html(cli, &input)
    } else {
        parse_as_xml(cli, &input)
    };

    let mut doc = match doc {
        Ok(d) => d,
        Err(msg) => {
            eprintln!("{filename}: {msg}");
            return EXIT_PARSE_ERROR;
        }
    };

    if cli.timing {
        let elapsed = start_parse.elapsed();
        eprintln!("Parsing took {elapsed:?}");
    }

    if cli.verbose && !doc.diagnostics.is_empty() {
        for diag in &doc.diagnostics {
            eprintln!("{filename}: {diag}");
        }
    }

    // -- Validation --------------------------------------------------------
    let mut exit_code = EXIT_SUCCESS;

    if cli.valid {
        let code = validate_dtd_internal(filename, &mut doc);
        if code > exit_code {
            exit_code = code;
        }
    }

    if let Some(ref dtd_file) = cli.dtdvalid {
        let code = validate_dtd_external(filename, &mut doc, dtd_file);
        if code > exit_code {
            exit_code = code;
        }
    }

    if let Some(ref rng_file) = cli.relaxng {
        let code = validate_relaxng_file(filename, &doc, rng_file);
        if code > exit_code {
            exit_code = code;
        }
    }

    if let Some(ref xsd_file) = cli.schema {
        let code = validate_xsd_file(filename, &doc, xsd_file);
        if code > exit_code {
            exit_code = code;
        }
    }

    if let Some(ref sch_file) = cli.schematron {
        let code = validate_schematron_file(filename, &doc, sch_file);
        if code > exit_code {
            exit_code = code;
        }
    }

    // -- XPath evaluation --------------------------------------------------
    if let Some(ref expr) = cli.xpath {
        evaluate_xpath(filename, &doc, expr);
    }

    // -- Debug tree --------------------------------------------------------
    if cli.debug {
        let debug_output = format_debug_tree(&doc);
        write_output(cli, &debug_output);
        return exit_code;
    }

    // -- Serialization / output --------------------------------------------
    if !cli.noout && cli.xpath.is_none() {
        let start_serial = Instant::now();

        let output_str = serialize_document(cli, &doc);
        write_output(cli, &output_str);

        if cli.timing {
            let elapsed = start_serial.elapsed();
            eprintln!("Serializing took {elapsed:?}");
        }
    }

    exit_code
}

// ---------------------------------------------------------------------------
// Input reading
// ---------------------------------------------------------------------------

/// Reads input from a file or stdin (when filename is `-`).
fn read_input(filename: &str) -> io::Result<String> {
    if filename == "-" {
        let mut buf = String::new();
        io::stdin().read_to_string(&mut buf)?;
        Ok(buf)
    } else {
        fs::read_to_string(filename)
    }
}

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

/// Parses input as XML with the configured options.
fn parse_as_xml(cli: &Cli, input: &str) -> Result<Document, String> {
    let opts = ParseOptions::default()
        .recover(cli.recover)
        .no_blanks(cli.noblanks);
    parser::parse_str_with_options(input, &opts).map_err(|e| e.to_string())
}

/// Parses input as HTML 4.01 with the configured options.
fn parse_as_html(cli: &Cli, input: &str) -> Result<Document, String> {
    let opts = xmloxide::html::HtmlParseOptions::default()
        .recover(cli.recover)
        .no_blanks(cli.noblanks);
    parse_html_with_options(input, &opts).map_err(|e| e.to_string())
}

/// Parses input as HTML5 (WHATWG parsing algorithm).
fn parse_as_html5(input: &str) -> Result<Document, String> {
    parse_html5(input).map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

/// Validates a document against its internal DTD (--valid).
fn validate_dtd_internal(filename: &str, doc: &mut Document) -> u8 {
    let dtd_text = extract_internal_dtd_subset(doc);
    if dtd_text.is_empty() {
        eprintln!("{filename}: no DTD found for validation");
        return EXIT_VALIDATION_ERROR;
    }

    match xmloxide::validation::dtd::parse_dtd(&dtd_text) {
        Ok(dtd) => {
            let result = xmloxide::validation::dtd::validate(doc, &dtd);
            print_validation_result(filename, &result)
        }
        Err(e) => {
            eprintln!("{filename}: failed to parse DTD: {e}");
            EXIT_VALIDATION_ERROR
        }
    }
}

/// Validates a document against an external DTD file (--dtdvalid).
fn validate_dtd_external(filename: &str, doc: &mut Document, dtd_file: &str) -> u8 {
    let dtd_content = match fs::read_to_string(dtd_file) {
        Ok(content) => content,
        Err(e) => {
            eprintln!("{dtd_file}: failed to read DTD: {e}");
            return EXIT_VALIDATION_ERROR;
        }
    };

    match xmloxide::validation::dtd::parse_dtd(&dtd_content) {
        Ok(dtd) => {
            let result = xmloxide::validation::dtd::validate(doc, &dtd);
            print_validation_result(filename, &result)
        }
        Err(e) => {
            eprintln!("{dtd_file}: failed to parse DTD: {e}");
            EXIT_VALIDATION_ERROR
        }
    }
}

/// Validates a document against a `RelaxNG` schema file (--relaxng).
fn validate_relaxng_file(filename: &str, doc: &Document, rng_file: &str) -> u8 {
    let schema_content = match fs::read_to_string(rng_file) {
        Ok(content) => content,
        Err(e) => {
            eprintln!("{rng_file}: failed to read RelaxNG schema: {e}");
            return EXIT_VALIDATION_ERROR;
        }
    };

    match xmloxide::validation::relaxng::parse_relaxng(&schema_content) {
        Ok(schema) => {
            let result = xmloxide::validation::relaxng::validate(doc, &schema);
            print_validation_result(filename, &result)
        }
        Err(e) => {
            eprintln!("{rng_file}: failed to parse RelaxNG schema: {e}");
            EXIT_VALIDATION_ERROR
        }
    }
}

/// Validates a document against an XML Schema (XSD) file (--schema).
fn validate_xsd_file(filename: &str, doc: &Document, xsd_file: &str) -> u8 {
    let schema_content = match fs::read_to_string(xsd_file) {
        Ok(content) => content,
        Err(e) => {
            eprintln!("{xsd_file}: failed to read XML Schema: {e}");
            return EXIT_VALIDATION_ERROR;
        }
    };

    match xmloxide::validation::xsd::parse_xsd(&schema_content) {
        Ok(schema) => {
            let result = xmloxide::validation::xsd::validate_xsd(doc, &schema);
            print_validation_result(filename, &result)
        }
        Err(e) => {
            eprintln!("{xsd_file}: failed to parse XML Schema: {e}");
            EXIT_VALIDATION_ERROR
        }
    }
}

/// Validates a document against an ISO Schematron schema file (--schematron).
fn validate_schematron_file(filename: &str, doc: &Document, sch_file: &str) -> u8 {
    let schema_content = match fs::read_to_string(sch_file) {
        Ok(content) => content,
        Err(e) => {
            eprintln!("{sch_file}: failed to read Schematron schema: {e}");
            return EXIT_VALIDATION_ERROR;
        }
    };

    match xmloxide::validation::schematron::parse_schematron(&schema_content) {
        Ok(schema) => {
            let result = xmloxide::validation::schematron::validate_schematron(doc, &schema);
            print_validation_result(filename, &result)
        }
        Err(e) => {
            eprintln!("{sch_file}: failed to parse Schematron schema: {e}");
            EXIT_VALIDATION_ERROR
        }
    }
}

/// Prints validation errors/warnings and returns the exit code.
fn print_validation_result(filename: &str, result: &xmloxide::validation::ValidationResult) -> u8 {
    for warning in &result.warnings {
        eprintln!("{filename}: validity warning: {warning}");
    }
    for error in &result.errors {
        eprintln!("{filename}: validity error: {error}");
    }
    if result.is_valid {
        eprintln!("{filename} validates");
        EXIT_SUCCESS
    } else {
        eprintln!("{filename} fails to validate");
        EXIT_VALIDATION_ERROR
    }
}

// ---------------------------------------------------------------------------
// XPath evaluation
// ---------------------------------------------------------------------------

/// Evaluates an `XPath` expression and prints the result to stdout.
fn evaluate_xpath(filename: &str, doc: &Document, expression: &str) {
    let context_node = doc.root_element().unwrap_or_else(|| doc.root());

    match xpath::evaluate(doc, context_node, expression) {
        Ok(value) => match &value {
            xpath::XPathValue::NodeSet(nodes) => {
                for &node in nodes {
                    match node {
                        xpath::XPathNode::Node(node_id) => {
                            let content = serialize_subtree(doc, node_id);
                            println!("{content}");
                        }
                        xpath::XPathNode::Attribute { owner, index } => {
                            // Print attribute nodes as name="value" with the
                            // value XML-escaped, matching libxml2's xmllint.
                            if let Some(attr) = doc.attributes(owner).get(index as usize) {
                                let value = escape_attribute_value(&attr.value);
                                match &attr.prefix {
                                    Some(prefix) => {
                                        println!("{prefix}:{}=\"{value}\"", attr.name);
                                    }
                                    None => println!("{}=\"{value}\"", attr.name),
                                }
                            }
                        }
                    }
                }
            }
            xpath::XPathValue::String(s) => {
                println!("{s}");
            }
            xpath::XPathValue::Number(n) => {
                println!("{n}");
            }
            xpath::XPathValue::Boolean(b) => {
                println!("{b}");
            }
        },
        Err(e) => {
            eprintln!("{filename}: XPath error: {e}");
        }
    }
}

/// Serializes a single node and its subtree to XML.
/// Escapes an attribute value for `name="value"` output (XML 1.0 §2.3).
fn escape_attribute_value(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(ch),
        }
    }
    out
}

fn serialize_subtree(doc: &Document, node_id: NodeId) -> String {
    let mut output = String::new();
    serialize_node_recursive(doc, node_id, &mut output);
    output
}

/// Recursively serializes a node to a string (for `XPath` output).
fn serialize_node_recursive(doc: &Document, id: NodeId, out: &mut String) {
    match &doc.node(id).kind {
        NodeKind::Element {
            name,
            prefix,
            attributes,
            ..
        } => {
            out.push('<');
            if let Some(pfx) = prefix {
                out.push_str(pfx);
                out.push(':');
            }
            out.push_str(name);
            for attr in attributes {
                out.push(' ');
                if let Some(pfx) = &attr.prefix {
                    out.push_str(pfx);
                    out.push(':');
                }
                out.push_str(&attr.name);
                out.push_str("=\"");
                out.push_str(&attr.value);
                out.push('"');
            }
            if doc.first_child(id).is_none() {
                out.push_str("/>");
            } else {
                out.push('>');
                for child in doc.children(id) {
                    serialize_node_recursive(doc, child, out);
                }
                out.push_str("</");
                if let Some(pfx) = prefix {
                    out.push_str(pfx);
                    out.push(':');
                }
                out.push_str(name);
                out.push('>');
            }
        }
        NodeKind::Text { content } => {
            out.push_str(content);
        }
        NodeKind::CData { content } => {
            out.push_str("<![CDATA[");
            out.push_str(content);
            out.push_str("]]>");
        }
        NodeKind::Comment { content } => {
            out.push_str("<!--");
            out.push_str(content);
            out.push_str("-->");
        }
        NodeKind::ProcessingInstruction { target, data } => {
            out.push_str("<?");
            out.push_str(target);
            if let Some(d) = data {
                out.push(' ');
                out.push_str(d);
            }
            out.push_str("?>");
        }
        NodeKind::EntityRef { name, .. } => {
            out.push('&');
            out.push_str(name);
            out.push(';');
        }
        NodeKind::DocumentType { .. } | NodeKind::Document => {}
    }
}

// ---------------------------------------------------------------------------
// Serialization
// ---------------------------------------------------------------------------

/// Serializes a document to string using the configured output mode.
fn serialize_document(cli: &Cli, doc: &Document) -> String {
    if cli.c14n || cli.exc_c14n {
        let opts = C14nOptions {
            with_comments: true,
            exclusive: cli.exc_c14n,
            inclusive_prefixes: Vec::new(),
        };
        let mut result = canonicalize(doc, &opts);
        result.push('\n');
        result
    } else {
        let mut output = serialize(doc);
        if cli.format {
            output = pretty_print(&output);
        }
        if let Some(ref enc) = cli.encode {
            output = update_encoding_declaration(&output, enc);
        }
        if !output.ends_with('\n') {
            output.push('\n');
        }
        output
    }
}

// ---------------------------------------------------------------------------
// Pretty printing
// ---------------------------------------------------------------------------

/// Simple pretty-printer that adds newlines and indentation between tags.
///
/// This operates on the serialized XML string, inserting newlines and
/// indentation at tag boundaries. It handles:
/// - Newlines after the XML declaration
/// - Indentation of nested elements
/// - Preserving text content inline with its parent element
fn pretty_print(xml: &str) -> String {
    let mut result = String::with_capacity(xml.len() * 2);
    let mut indent_level: usize = 0;
    let indent_str = "  ";

    // Split the XML into tokens: tags (starting with '<' and ending with '>')
    // and text content between tags.
    let tokens = tokenize_xml(xml);

    let mut i = 0;
    while i < tokens.len() {
        let token = &tokens[i];

        if token.starts_with("<?") {
            // Processing instruction / XML declaration
            result.push_str(token);
            result.push('\n');
        } else if token.starts_with("<!--") {
            // Comment
            push_indent(&mut result, indent_level, indent_str);
            result.push_str(token);
            result.push('\n');
        } else if token.starts_with("<!") {
            // DOCTYPE or other declaration
            push_indent(&mut result, indent_level, indent_str);
            result.push_str(token);
            result.push('\n');
        } else if token.starts_with("</") {
            // Closing tag
            indent_level = indent_level.saturating_sub(1);
            push_indent(&mut result, indent_level, indent_str);
            result.push_str(token);
            result.push('\n');
        } else if token.starts_with('<') {
            let extra = format_open_tag(&tokens, i, &mut result, &mut indent_level, indent_str);
            i += extra;
        } else {
            // Text content on its own
            let trimmed = token.trim();
            if !trimmed.is_empty() {
                push_indent(&mut result, indent_level, indent_str);
                result.push_str(trimmed);
                result.push('\n');
            }
        }

        i += 1;
    }

    result
}

/// Splits XML into tokens of tags and text content.
fn tokenize_xml(xml: &str) -> Vec<String> {
    let mut tokens: Vec<String> = Vec::new();
    let mut current = String::new();

    for ch in xml.chars() {
        if ch == '<' {
            if !current.is_empty() {
                tokens.push(current.clone());
                current.clear();
            }
            current.push(ch);
        } else if ch == '>' {
            current.push(ch);
            tokens.push(current.clone());
            current.clear();
        } else {
            current.push(ch);
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }

    tokens
}

/// Handles formatting of an opening tag token, including inline text
/// optimization where `<tag>text</tag>` stays on one line.
///
/// Returns the number of extra tokens consumed (for the caller to skip).
fn format_open_tag(
    tokens: &[String],
    i: usize,
    result: &mut String,
    indent_level: &mut usize,
    indent_str: &str,
) -> usize {
    let token = &tokens[i];
    let is_self_closing = token.ends_with("/>");

    // Check if the next token is text content (not another tag)
    let next_is_text = tokens.get(i + 1).is_some_and(|t| !t.starts_with('<'));

    // Check if it's like <tag>text</tag> (inline text content)
    let is_inline_text = next_is_text && tokens.get(i + 2).is_some_and(|t| t.starts_with("</"));

    if is_self_closing {
        push_indent(result, *indent_level, indent_str);
        result.push_str(token);
        result.push('\n');
        0
    } else if is_inline_text {
        // Output <tag>text</tag> on one line
        push_indent(result, *indent_level, indent_str);
        result.push_str(token);
        result.push_str(&tokens[i + 1]); // text
        result.push_str(&tokens[i + 2]); // </tag>
        result.push('\n');
        2 // skip text + closing tag
    } else {
        push_indent(result, *indent_level, indent_str);
        result.push_str(token);
        if next_is_text {
            result.push_str(&tokens[i + 1]);
            *indent_level += 1;
            1 // skip the text token
        } else {
            result.push('\n');
            *indent_level += 1;
            0
        }
    }
}

/// Writes indentation to the output string.
fn push_indent(out: &mut String, level: usize, indent: &str) {
    for _ in 0..level {
        out.push_str(indent);
    }
}

// ---------------------------------------------------------------------------
// Encoding
// ---------------------------------------------------------------------------

/// Updates the encoding attribute in an XML declaration, if present.
fn update_encoding_declaration(xml: &str, new_encoding: &str) -> String {
    if let Some(decl_end) = xml.find("?>") {
        let decl = &xml[..decl_end];
        if let Some(enc_start) = decl.find("encoding=\"") {
            let after_enc = &decl[enc_start + 10..];
            if let Some(enc_end) = after_enc.find('"') {
                let mut result = String::with_capacity(xml.len());
                result.push_str(&xml[..enc_start + 10]);
                result.push_str(new_encoding);
                result.push_str(&xml[enc_start + 10 + enc_end..]);
                return result;
            }
        }
    }
    xml.to_string()
}

// ---------------------------------------------------------------------------
// Debug tree
// ---------------------------------------------------------------------------

/// Produces a textual debug representation of the document tree.
///
/// The format resembles libxml2's `--debug` output: each node is printed
/// with its type and content, indented to show the tree structure.
fn format_debug_tree(doc: &Document) -> String {
    let mut output = String::new();
    output.push_str("DOCUMENT\n");
    for child in doc.children(doc.root()) {
        format_debug_node(doc, child, 1, &mut output);
    }
    output
}

/// Recursively formats a node for debug output.
fn format_debug_node(doc: &Document, id: NodeId, depth: usize, out: &mut String) {
    let indent: String = "  ".repeat(depth);

    match &doc.node(id).kind {
        NodeKind::Element {
            name,
            prefix,
            namespace,
            attributes,
        } => {
            let qname = match prefix {
                Some(pfx) => format!("{pfx}:{name}"),
                None => name.clone(),
            };
            out.push_str(&indent);
            out.push_str("ELEMENT ");
            out.push_str(&qname);
            if let Some(ns) = namespace {
                let _ = write!(out, " ns={ns}");
            }
            out.push('\n');
            for attr in attributes {
                out.push_str(&indent);
                out.push_str("  ATTRIBUTE ");
                if let Some(pfx) = &attr.prefix {
                    out.push_str(pfx);
                    out.push(':');
                }
                out.push_str(&attr.name);
                out.push('=');
                out.push_str(&attr.value);
                out.push('\n');
            }
            for child in doc.children(id) {
                format_debug_node(doc, child, depth + 1, out);
            }
        }
        NodeKind::Text { content } => {
            out.push_str(&indent);
            out.push_str("TEXT ");
            // Show the text content, replacing newlines for readability
            let display = content.replace('\n', "\\n");
            out.push_str(&display);
            out.push('\n');
        }
        NodeKind::CData { content } => {
            out.push_str(&indent);
            out.push_str("CDATA ");
            out.push_str(content);
            out.push('\n');
        }
        NodeKind::Comment { content } => {
            out.push_str(&indent);
            out.push_str("COMMENT ");
            out.push_str(content);
            out.push('\n');
        }
        NodeKind::ProcessingInstruction { target, data } => {
            out.push_str(&indent);
            out.push_str("PI ");
            out.push_str(target);
            if let Some(d) = data {
                out.push(' ');
                out.push_str(d);
            }
            out.push('\n');
        }
        NodeKind::EntityRef { name, .. } => {
            out.push_str(&indent);
            out.push_str("ENTITY_REF ");
            out.push_str(name);
            out.push('\n');
        }
        NodeKind::DocumentType {
            name,
            system_id,
            public_id,
            ..
        } => {
            out.push_str(&indent);
            out.push_str("DOCTYPE ");
            out.push_str(name);
            if let Some(pub_id) = public_id {
                let _ = write!(out, " PUBLIC \"{pub_id}\"");
            }
            if let Some(sys_id) = system_id {
                let _ = write!(out, " SYSTEM \"{sys_id}\"");
            }
            out.push('\n');
        }
        NodeKind::Document => {
            out.push_str(&indent);
            out.push_str("DOCUMENT\n");
        }
    }
}

// ---------------------------------------------------------------------------
// Output writing
// ---------------------------------------------------------------------------

/// Writes output to stdout or to the file specified by --output.
fn write_output(cli: &Cli, content: &str) {
    if let Some(ref output_file) = cli.output {
        if let Err(e) = fs::write(output_file, content) {
            eprintln!("{output_file}: failed to write: {e}");
        }
    } else {
        print!("{content}");
        // Flush stdout to ensure output is complete, especially when piped.
        let _ = io::stdout().flush();
    }
}

// ---------------------------------------------------------------------------
// DTD extraction helper
// ---------------------------------------------------------------------------

/// Extracts the internal DTD subset text from the document, if any.
///
/// Looks for a `DocumentType` node and attempts to extract a minimal DTD from
/// the document's content model. This is a best-effort approach -- a full
/// implementation would capture the internal subset during parsing.
fn extract_internal_dtd_subset(doc: &Document) -> String {
    // Walk the document's top-level children looking for a DocumentType node.
    for child in doc.children(doc.root()) {
        if matches!(doc.node(child).kind, NodeKind::DocumentType { .. }) {
            // We found a DOCTYPE but the current tree representation doesn't
            // store the internal subset text. Return empty to indicate that
            // the DTD can't be extracted from the tree alone.
            return String::new();
        }
    }
    String::new()
}
