/*
 * xmloxide.h — C API for xmloxide
 *
 * A memory-safe XML parsing library implemented in Rust.
 *
 * All returned strings are caller-owned and must be freed with
 * xmloxide_free_string(). Document and XPath result pointers must be
 * freed with their respective free functions.
 *
 * Error handling: functions that can fail return NULL (for pointers)
 * or 0 (for node ids). Call xmloxide_last_error() to retrieve the
 * error message for the most recent failure on the current thread.
 *
 * Thread safety: Unlike libxml2, xmloxide requires no global
 * initialization or cleanup. Each document is independent and may be
 * used from any thread. The last-error message is stored in thread-local
 * storage, so each thread has its own error state. A single document
 * must not be accessed concurrently from multiple threads without
 * external synchronization.
 */

#ifndef XMLOXIDE_H
#define XMLOXIDE_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* ---------- Opaque types ---------- */

/** Opaque XML document handle. */
typedef struct xmloxide_document xmloxide_document;

/** Opaque XPath result handle. */
typedef struct xmloxide_xpath_value xmloxide_xpath_value;

/** Opaque DTD handle. */
typedef struct xmloxide_dtd xmloxide_dtd;

/** Opaque RelaxNG schema handle. */
typedef struct xmloxide_relaxng_schema xmloxide_relaxng_schema;

/** Opaque XSD schema handle. */
typedef struct xmloxide_xsd_schema xmloxide_xsd_schema;

/** Opaque Schematron schema handle. */
typedef struct xmloxide_schematron_schema xmloxide_schematron_schema;

/** Opaque validation result handle. */
typedef struct xmloxide_validation_result xmloxide_validation_result;

/** Opaque XML Catalog handle. */
typedef struct xmloxide_catalog xmloxide_catalog;

/** Opaque push parser handle. */
typedef struct xmloxide_push_parser xmloxide_push_parser;

/** Opaque XML reader handle. */
typedef struct xmloxide_reader xmloxide_reader;

/* ---------- Node type constants ---------- */

#define XMLOXIDE_NODE_ELEMENT       1
#define XMLOXIDE_NODE_TEXT          3
#define XMLOXIDE_NODE_CDATA        4
#define XMLOXIDE_NODE_ENTITY_REF   5
#define XMLOXIDE_NODE_PI           7
#define XMLOXIDE_NODE_COMMENT      8
#define XMLOXIDE_NODE_DOCUMENT     9
#define XMLOXIDE_NODE_DOCUMENT_TYPE 10

/* ---------- XPath result type constants ---------- */

#define XMLOXIDE_XPATH_NODESET  1
#define XMLOXIDE_XPATH_BOOLEAN  2
#define XMLOXIDE_XPATH_NUMBER   3
#define XMLOXIDE_XPATH_STRING   4

/* ---------- Error severity constants ---------- */

#define XMLOXIDE_ERR_WARNING  0
#define XMLOXIDE_ERR_ERROR    1
#define XMLOXIDE_ERR_FATAL    2

/* ---------- Error handling ---------- */

/**
 * Returns the last error message, or NULL if no error occurred.
 *
 * The returned string is owned by the library and must NOT be freed.
 * It is valid until the next xmloxide FFI call on the same thread.
 */
const char *xmloxide_last_error(void);

/**
 * Returns the line number where the last error occurred, or 0 if unknown.
 */
uint32_t xmloxide_last_error_line(void);

/**
 * Returns the column number where the last error occurred, or 0 if unknown.
 */
uint32_t xmloxide_last_error_column(void);

/**
 * Returns the severity of the last error.
 * Returns XMLOXIDE_ERR_WARNING (0), XMLOXIDE_ERR_ERROR (1),
 * or XMLOXIDE_ERR_FATAL (2). Returns -1 if no error occurred.
 */
int32_t xmloxide_last_error_severity(void);

/* ---------- Document lifecycle ---------- */

/**
 * Parses a null-terminated UTF-8 XML string into a document.
 *
 * Returns a document pointer on success, or NULL on failure.
 * The returned document must be freed with xmloxide_free_doc().
 */
xmloxide_document *xmloxide_parse_str(const char *input);

/**
 * Parses raw bytes as XML, with automatic encoding detection.
 *
 * Returns a document pointer on success, or NULL on failure.
 * The returned document must be freed with xmloxide_free_doc().
 */
xmloxide_document *xmloxide_parse_bytes(const uint8_t *data, size_t len);

/**
 * Parses an HTML string into a document.
 *
 * Returns a document pointer on success, or NULL on failure.
 * The returned document must be freed with xmloxide_free_doc().
 */
xmloxide_document *xmloxide_parse_html(const char *input);

/**
 * Parses an HTML5 string using the WHATWG parsing algorithm.
 *
 * Returns a document pointer on success, or NULL on failure.
 * The returned document must be freed with xmloxide_free_doc().
 */
xmloxide_document *xmloxide_parse_html5(const char *input);

/**
 * Parses an HTML5 fragment with a context element (the innerHTML algorithm).
 *
 * context_element is the tag name of the context (e.g., "body", "div", "table").
 * Returns a document pointer on success, or NULL on failure.
 * The returned document must be freed with xmloxide_free_doc().
 */
xmloxide_document *xmloxide_parse_html5_fragment(const char *input,
                                                  const char *context_element);

/**
 * Parses an XML file from a filesystem path.
 *
 * Returns a document pointer on success, or NULL on failure.
 * The returned document must be freed with xmloxide_free_doc().
 */
xmloxide_document *xmloxide_parse_file(const char *path);

/**
 * Frees a document previously returned by a parse function.
 * Passing NULL is safe and does nothing.
 */
void xmloxide_free_doc(xmloxide_document *doc);

/* ---------- Document properties ---------- */

/**
 * Returns the XML version string (e.g., "1.0"), or NULL if not declared.
 * The returned string must be freed with xmloxide_free_string().
 */
char *xmloxide_doc_version(const xmloxide_document *doc);

/**
 * Returns the encoding string (e.g., "UTF-8"), or NULL if not declared.
 * The returned string must be freed with xmloxide_free_string().
 */
char *xmloxide_doc_encoding(const xmloxide_document *doc);

/* ---------- Document diagnostics ---------- */

/**
 * Returns the number of parse diagnostics (warnings + recovered errors)
 * on a document. Returns 0 if the document has no diagnostics.
 */
size_t xmloxide_doc_diagnostic_count(const xmloxide_document *doc);

/**
 * Returns the error message of the diagnostic at the given index.
 * Returns NULL if out of range.
 * The returned string must be freed with xmloxide_free_string().
 */
char *xmloxide_doc_diagnostic_message(const xmloxide_document *doc, size_t index);

/** Returns the line number of the diagnostic at the given index (0 if unknown). */
uint32_t xmloxide_doc_diagnostic_line(const xmloxide_document *doc, size_t index);

/** Returns the column number of the diagnostic at the given index (0 if unknown). */
uint32_t xmloxide_doc_diagnostic_column(const xmloxide_document *doc, size_t index);

/**
 * Returns the severity of the diagnostic at the given index.
 * Returns XMLOXIDE_ERR_WARNING, XMLOXIDE_ERR_ERROR, or XMLOXIDE_ERR_FATAL.
 * Returns -1 if out of range.
 */
int32_t xmloxide_doc_diagnostic_severity(const xmloxide_document *doc, size_t index);

/* ---------- Tree navigation ---------- */

/*
 * Node IDs are uint32_t values. A value of 0 means "no node"
 * (invalid/missing).
 */

/** Returns the document root node id. */
uint32_t xmloxide_doc_root(const xmloxide_document *doc);

/** Returns the root element of the document, or 0 if none. */
uint32_t xmloxide_doc_root_element(const xmloxide_document *doc);

/** Returns the parent of a node, or 0 if none. */
uint32_t xmloxide_node_parent(const xmloxide_document *doc, uint32_t node);

/** Returns the first child of a node, or 0 if none. */
uint32_t xmloxide_node_first_child(const xmloxide_document *doc, uint32_t node);

/** Returns the last child of a node, or 0 if none. */
uint32_t xmloxide_node_last_child(const xmloxide_document *doc, uint32_t node);

/** Returns the next sibling of a node, or 0 if none. */
uint32_t xmloxide_node_next_sibling(const xmloxide_document *doc, uint32_t node);

/** Returns the previous sibling of a node, or 0 if none. */
uint32_t xmloxide_node_prev_sibling(const xmloxide_document *doc, uint32_t node);

/* ---------- Node inspection ---------- */

/**
 * Returns the node type as an integer constant.
 * Returns -1 if the document or node is invalid.
 */
int32_t xmloxide_node_type(const xmloxide_document *doc, uint32_t node);

/**
 * Returns the name of a node (element local name or PI target).
 * Returns NULL for node types that have no name.
 * The returned string must be freed with xmloxide_free_string().
 */
char *xmloxide_node_name(const xmloxide_document *doc, uint32_t node);

/**
 * Returns the direct text content of a text, comment, CDATA, or PI node.
 * Returns NULL for element and document nodes.
 * The returned string must be freed with xmloxide_free_string().
 */
char *xmloxide_node_text(const xmloxide_document *doc, uint32_t node);

/**
 * Returns the concatenated text content of a node and all descendants.
 * The returned string must be freed with xmloxide_free_string().
 */
char *xmloxide_node_text_content(const xmloxide_document *doc, uint32_t node);

/**
 * Returns the namespace URI of an element node, or NULL if none.
 * The returned string must be freed with xmloxide_free_string().
 */
char *xmloxide_node_namespace(const xmloxide_document *doc, uint32_t node);

/**
 * Returns the namespace prefix of an element node (e.g., "svg" for <svg:rect>).
 * Returns NULL if no prefix. The returned string must be freed with
 * xmloxide_free_string().
 */
char *xmloxide_node_prefix(const xmloxide_document *doc, uint32_t node);

/**
 * Returns the value of an attribute by name on an element node.
 * Returns NULL if the attribute is not present.
 * The returned string must be freed with xmloxide_free_string().
 */
char *xmloxide_node_attribute(const xmloxide_document *doc, uint32_t node,
                              const char *name);

/**
 * Returns the number of attributes on an element node.
 * Returns 0 for non-element nodes.
 */
size_t xmloxide_node_attribute_count(const xmloxide_document *doc, uint32_t node);

/**
 * Returns the name of the attribute at the given index.
 * Returns NULL if the index is out of range.
 * The returned string must be freed with xmloxide_free_string().
 */
char *xmloxide_node_attribute_name_at(const xmloxide_document *doc,
                                      uint32_t node, size_t index);

/**
 * Returns the value of the attribute at the given index.
 * Returns NULL if the index is out of range.
 * The returned string must be freed with xmloxide_free_string().
 */
char *xmloxide_node_attribute_value_at(const xmloxide_document *doc,
                                       uint32_t node, size_t index);

/* ---------- Tree mutation ---------- */

/**
 * Creates a new element node and returns its id (0 on failure).
 * The node is detached — use xmloxide_append_child() to add it to the tree.
 */
uint32_t xmloxide_create_element(xmloxide_document *doc, const char *name);

/**
 * Creates a new text node and returns its id (0 on failure).
 */
uint32_t xmloxide_create_text(xmloxide_document *doc, const char *content);

/**
 * Creates a new comment node and returns its id (0 on failure).
 */
uint32_t xmloxide_create_comment(xmloxide_document *doc, const char *content);

/**
 * Appends a child node to a parent. Returns 1 on success, 0 on failure.
 */
int32_t xmloxide_append_child(xmloxide_document *doc, uint32_t parent,
                              uint32_t child);

/**
 * Removes a node from the tree. Returns 1 on success, 0 on failure.
 * The node remains in the arena but is detached from the tree.
 */
int32_t xmloxide_remove_node(xmloxide_document *doc, uint32_t node);

/**
 * Clones a node (and optionally its descendants). Returns the new node id.
 * Set deep=1 for a deep clone, deep=0 for a shallow clone.
 * Returns 0 on failure.
 */
uint32_t xmloxide_clone_node(xmloxide_document *doc, uint32_t node, int32_t deep);

/**
 * Sets an attribute on an element node. Returns 1 on success, 0 on failure.
 * If the attribute already exists, its value is updated.
 */
int32_t xmloxide_set_attribute(xmloxide_document *doc, uint32_t node,
                               const char *name, const char *value);

/**
 * Sets the text content of a node. Returns 1 on success, 0 on failure.
 * For text/CDATA/comment nodes, updates content directly.
 * For element nodes, removes all children and replaces with a text node.
 */
int32_t xmloxide_set_text_content(xmloxide_document *doc, uint32_t node,
                                  const char *content);

/**
 * Inserts a node before a reference sibling. Returns 1 on success, 0 on failure.
 */
int32_t xmloxide_insert_before(xmloxide_document *doc, uint32_t reference,
                               uint32_t new_child);

/**
 * Inserts a node after a reference sibling. Returns 1 on success, 0 on failure.
 */
int32_t xmloxide_insert_after(xmloxide_document *doc, uint32_t reference,
                              uint32_t new_child);

/**
 * Replaces a node in the tree with another. Returns 1 on success, 0 on failure.
 * The old node is detached and the new node takes its position.
 */
int32_t xmloxide_replace_node(xmloxide_document *doc, uint32_t old_node,
                              uint32_t new_node);

/**
 * Removes an attribute by name from an element node.
 * Returns 1 if removed, 0 if not found or not an element.
 */
int32_t xmloxide_remove_attribute(xmloxide_document *doc, uint32_t node,
                                  const char *name);

/**
 * Creates a new processing instruction node and returns its id (0 on failure).
 * data may be NULL.
 */
uint32_t xmloxide_create_pi(xmloxide_document *doc, const char *target,
                            const char *data);

/**
 * Renames an element node. Returns 1 on success, 0 on failure.
 */
int32_t xmloxide_rename_element(xmloxide_document *doc, uint32_t node,
                                const char *new_name);

/**
 * Returns the element with the given ID attribute, or 0 if not found.
 * The document's id_map must be populated first (typically via DTD validation).
 */
uint32_t xmloxide_element_by_id(const xmloxide_document *doc, const char *id);

/* ---------- Serialization ---------- */

/**
 * Serializes a document to an XML string.
 * Returns a caller-owned C string that must be freed with
 * xmloxide_free_string(). Returns NULL on failure.
 */
char *xmloxide_serialize(const xmloxide_document *doc);

/**
 * Serializes a document to a pretty-printed XML string with two-space indent.
 * Returns a caller-owned C string that must be freed with
 * xmloxide_free_string(). Returns NULL on failure.
 */
char *xmloxide_serialize_pretty(const xmloxide_document *doc);

/**
 * Serializes a document to a pretty-printed XML string with a custom indent.
 * indent_str is the string used for each level (e.g., "\t" or "    ").
 * Returns a caller-owned C string that must be freed with
 * xmloxide_free_string(). Returns NULL on failure.
 */
char *xmloxide_serialize_pretty_custom(const xmloxide_document *doc,
                                       const char *indent_str);

/**
 * Serializes a document to an HTML string.
 * Returns a caller-owned C string that must be freed with
 * xmloxide_free_string(). Returns NULL on failure.
 */
char *xmloxide_serialize_html(const xmloxide_document *doc);

/**
 * Serializes a document to an HTML5 string (WHATWG algorithm).
 * Returns a caller-owned C string that must be freed with
 * xmloxide_free_string(). Returns NULL on failure.
 */
char *xmloxide_serialize_html5(const xmloxide_document *doc);

/* ---------- Validation ---------- */

/**
 * Parses a DTD from a null-terminated UTF-8 string.
 * Returns a DTD pointer on success, or NULL on failure.
 * The returned DTD must be freed with xmloxide_free_dtd().
 */
xmloxide_dtd *xmloxide_parse_dtd(const char *input);

/** Frees a DTD. Passing NULL is safe and does nothing. */
void xmloxide_free_dtd(xmloxide_dtd *dtd);

/**
 * Validates a document against a DTD.
 * Note: DTD validation may populate the document's id_map (requires mutable doc).
 * Returns a validation result that must be freed with
 * xmloxide_free_validation_result().
 */
xmloxide_validation_result *xmloxide_validate_dtd(xmloxide_document *doc,
                                                  const xmloxide_dtd *dtd);

/**
 * Parses a RelaxNG schema from a null-terminated UTF-8 XML string.
 * Returns a schema pointer on success, or NULL on failure.
 * The returned schema must be freed with xmloxide_free_relaxng().
 */
xmloxide_relaxng_schema *xmloxide_parse_relaxng(const char *input);

/** Frees a RelaxNG schema. Passing NULL is safe and does nothing. */
void xmloxide_free_relaxng(xmloxide_relaxng_schema *schema);

/**
 * Validates a document against a RelaxNG schema.
 * Returns a validation result that must be freed with
 * xmloxide_free_validation_result().
 */
xmloxide_validation_result *xmloxide_validate_relaxng(const xmloxide_document *doc,
                                                      const xmloxide_relaxng_schema *schema);

/**
 * Parses an XSD schema from a null-terminated UTF-8 XML string.
 * Returns a schema pointer on success, or NULL on failure.
 * The returned schema must be freed with xmloxide_free_xsd().
 */
xmloxide_xsd_schema *xmloxide_parse_xsd(const char *input);

/** Frees an XSD schema. Passing NULL is safe and does nothing. */
void xmloxide_free_xsd(xmloxide_xsd_schema *schema);

/**
 * Validates a document against an XSD schema.
 * Returns a validation result that must be freed with
 * xmloxide_free_validation_result().
 */
xmloxide_validation_result *xmloxide_validate_xsd(const xmloxide_document *doc,
                                                  const xmloxide_xsd_schema *schema);

/**
 * Parses an ISO Schematron schema from a null-terminated UTF-8 XML string.
 * Returns a schema pointer on success, or NULL on failure.
 * The returned schema must be freed with xmloxide_free_schematron().
 */
xmloxide_schematron_schema *xmloxide_parse_schematron(const char *input);

/** Frees a Schematron schema. Passing NULL is safe and does nothing. */
void xmloxide_free_schematron(xmloxide_schematron_schema *schema);

/**
 * Validates a document against an ISO Schematron schema.
 * Returns a validation result that must be freed with
 * xmloxide_free_validation_result().
 */
xmloxide_validation_result *xmloxide_validate_schematron(
    const xmloxide_document *doc,
    const xmloxide_schematron_schema *schema);

/**
 * Validates a document against a Schematron schema using a specific phase.
 * phase is the name of the phase to activate (NULL for all patterns).
 * Returns a validation result that must be freed with
 * xmloxide_free_validation_result().
 */
xmloxide_validation_result *xmloxide_validate_schematron_with_phase(
    const xmloxide_document *doc,
    const xmloxide_schematron_schema *schema,
    const char *phase);

/**
 * Returns whether the validation result indicates a valid document.
 * Returns 1 for valid, 0 for invalid or NULL.
 */
int32_t xmloxide_validation_is_valid(const xmloxide_validation_result *result);

/**
 * Returns the number of validation errors.
 * Returns 0 if the result is NULL.
 */
size_t xmloxide_validation_error_count(const xmloxide_validation_result *result);

/**
 * Returns the error message at the given index.
 * Returns NULL if the index is out of range.
 * The returned string must be freed with xmloxide_free_string().
 */
char *xmloxide_validation_error_message(const xmloxide_validation_result *result,
                                        size_t index);

/**
 * Returns the number of validation warnings.
 * Returns 0 if the result is NULL.
 */
size_t xmloxide_validation_warning_count(const xmloxide_validation_result *result);

/**
 * Returns the warning message at the given index.
 * Returns NULL if the index is out of range.
 * The returned string must be freed with xmloxide_free_string().
 */
char *xmloxide_validation_warning_message(const xmloxide_validation_result *result,
                                          size_t index);

/**
 * Frees a validation result. Passing NULL is safe and does nothing.
 */
void xmloxide_free_validation_result(xmloxide_validation_result *result);

/* ---------- XPath ---------- */

/**
 * Evaluates an XPath expression against a context node.
 *
 * Returns a pointer to the result on success, or NULL on failure.
 * Use context_node=0 to use the document root as context.
 * The returned result must be freed with xmloxide_xpath_free_result().
 */
xmloxide_xpath_value *xmloxide_xpath_eval(const xmloxide_document *doc,
                                          uint32_t context_node,
                                          const char *expr);

/**
 * Returns the type of an XPath result.
 * Returns one of the XMLOXIDE_XPATH_* constants, or -1 on error.
 */
int32_t xmloxide_xpath_result_type(const xmloxide_xpath_value *result);

/**
 * Returns the boolean value of an XPath result.
 * Converts non-boolean results using XPath type coercion rules.
 */
int32_t xmloxide_xpath_result_boolean(const xmloxide_xpath_value *result);

/**
 * Returns the numeric value of an XPath result.
 * Converts non-number results using XPath type coercion rules.
 */
double xmloxide_xpath_result_number(const xmloxide_xpath_value *result);

/**
 * Returns the string value of an XPath result.
 * Converts non-string results using XPath type coercion rules.
 * The returned string must be freed with xmloxide_free_string().
 */
char *xmloxide_xpath_result_string(const xmloxide_xpath_value *result);

/** Returns the number of nodes in an XPath nodeset result. */
size_t xmloxide_xpath_nodeset_count(const xmloxide_xpath_value *result);

/**
 * Returns the node id at the given index in an XPath nodeset result.
 * For attribute nodes, returns the id of the owner element (use the
 * item_is_attribute / item_attr_name / item_attr_value accessors to
 * inspect the attribute itself).
 * Returns 0 if the result is not a nodeset or the index is out of bounds.
 */
uint32_t xmloxide_xpath_nodeset_item(const xmloxide_xpath_value *result,
                                     size_t index);

/**
 * Returns 1 if the nodeset entry at the given index is an attribute node,
 * 0 otherwise (including out-of-bounds and non-nodeset results).
 */
int xmloxide_xpath_nodeset_item_is_attribute(const xmloxide_xpath_value *result,
                                             size_t index);

/**
 * Returns the qualified name (prefix:local) of the attribute at the given
 * index in a nodeset result, or NULL if the entry is not an attribute.
 * `doc` must be the document the result was evaluated against.
 * The returned string must be freed with xmloxide_free_string().
 */
char *xmloxide_xpath_nodeset_item_attr_name(const xmloxide_document *doc,
                                            const xmloxide_xpath_value *result,
                                            size_t index);

/**
 * Returns the value of the attribute at the given index in a nodeset
 * result, or NULL if the entry is not an attribute.
 * `doc` must be the document the result was evaluated against.
 * The returned string must be freed with xmloxide_free_string().
 */
char *xmloxide_xpath_nodeset_item_attr_value(const xmloxide_document *doc,
                                             const xmloxide_xpath_value *result,
                                             size_t index);

/**
 * Frees an XPath result previously returned by xmloxide_xpath_eval().
 * Passing NULL is safe and does nothing.
 */
void xmloxide_xpath_free_result(xmloxide_xpath_value *result);

/* ---------- Canonical XML (C14N) ---------- */

/**
 * Canonicalizes a document using inclusive C14N with comments.
 * Returns a caller-owned C string that must be freed with
 * xmloxide_free_string(). Returns NULL on failure.
 */
char *xmloxide_canonicalize(const xmloxide_document *doc);

/**
 * Canonicalizes a document with options.
 * with_comments: 1 to include comments, 0 to strip.
 * exclusive: 1 for exclusive C14N, 0 for inclusive.
 */
char *xmloxide_canonicalize_opts(const xmloxide_document *doc,
                                 int32_t with_comments, int32_t exclusive);

/**
 * Canonicalizes a subtree rooted at the given node.
 */
char *xmloxide_canonicalize_subtree(const xmloxide_document *doc,
                                    uint32_t node, int32_t with_comments,
                                    int32_t exclusive);

/* ---------- XInclude ---------- */

/**
 * Processes XInclude elements in a document using file-based resolution.
 * Returns the number of successful inclusions, or -1 on failure.
 * Errors are stored in the thread-local error (retrievable via xmloxide_last_error).
 */
int32_t xmloxide_process_xincludes(xmloxide_document *doc);

/* ---------- XML Catalogs ---------- */

/**
 * Parses an XML Catalog from a null-terminated UTF-8 XML string.
 * Returns a catalog pointer on success, or NULL on failure.
 * The returned catalog must be freed with xmloxide_free_catalog().
 */
xmloxide_catalog *xmloxide_parse_catalog(const char *input);

/** Frees a catalog. Passing NULL is safe and does nothing. */
void xmloxide_free_catalog(xmloxide_catalog *catalog);

/**
 * Resolves a system identifier using the catalog.
 * Returns a caller-owned URI string, or NULL if not found.
 */
char *xmloxide_catalog_resolve_system(const xmloxide_catalog *catalog,
                                      const char *system_id);

/**
 * Resolves a public identifier using the catalog.
 * Returns a caller-owned URI string, or NULL if not found.
 */
char *xmloxide_catalog_resolve_public(const xmloxide_catalog *catalog,
                                      const char *public_id);

/**
 * Resolves a URI using the catalog.
 * Returns a caller-owned URI string, or NULL if not found.
 */
char *xmloxide_catalog_resolve_uri(const xmloxide_catalog *catalog,
                                   const char *uri);

/* ---------- Push parser (incremental) ---------- */

/**
 * Creates a new push parser with default options.
 * The returned parser must be consumed via xmloxide_push_parser_finish()
 * or freed with xmloxide_push_parser_free().
 */
xmloxide_push_parser *xmloxide_push_parser_new(void);

/**
 * Feeds a chunk of raw bytes into the push parser.
 * Data can be split at arbitrary byte boundaries.
 */
void xmloxide_push_parser_push(xmloxide_push_parser *parser,
                                const uint8_t *data, size_t len);

/**
 * Finalizes parsing and returns the constructed document.
 *
 * This CONSUMES the parser — the parser pointer becomes invalid after
 * this call. Do NOT call xmloxide_push_parser_free() after finish.
 *
 * Returns a document pointer on success, or NULL on failure.
 * The returned document must be freed with xmloxide_free_doc().
 */
xmloxide_document *xmloxide_push_parser_finish(xmloxide_push_parser *parser);

/**
 * Returns the number of bytes currently buffered in the push parser.
 */
size_t xmloxide_push_parser_buffered_bytes(const xmloxide_push_parser *parser);

/**
 * Resets the push parser, discarding all buffered data.
 * The parser can then be reused for a new document.
 */
void xmloxide_push_parser_reset(xmloxide_push_parser *parser);

/**
 * Frees a push parser without finishing it.
 * Use this to discard a parser whose data you no longer need.
 * Passing NULL is safe. Do NOT call after finish().
 */
void xmloxide_push_parser_free(xmloxide_push_parser *parser);

/* ---------- XmlReader (pull-based streaming) ---------- */

/*
 * Reader node type constants (matching libxml2's xmlReaderTypes).
 */
#define XMLOXIDE_READER_NONE             0
#define XMLOXIDE_READER_ELEMENT          1
#define XMLOXIDE_READER_ATTRIBUTE        2
#define XMLOXIDE_READER_TEXT             3
#define XMLOXIDE_READER_CDATA            4
#define XMLOXIDE_READER_PI               7
#define XMLOXIDE_READER_COMMENT          8
#define XMLOXIDE_READER_DOCUMENT_TYPE   10
#define XMLOXIDE_READER_WHITESPACE      13
#define XMLOXIDE_READER_END_ELEMENT     15
#define XMLOXIDE_READER_XML_DECLARATION 17
#define XMLOXIDE_READER_END_DOCUMENT    (-1)

/**
 * Creates a new XmlReader from a null-terminated UTF-8 string.
 * Returns an opaque reader pointer, or NULL on failure.
 * The reader must be freed with xmloxide_reader_free().
 */
xmloxide_reader *xmloxide_reader_new(const char *input);

/**
 * Advances the reader to the next node.
 * Returns 1 if a node was read, 0 at end of document, -1 on error.
 */
int32_t xmloxide_reader_read(xmloxide_reader *reader);

/**
 * Returns the node type of the current node.
 * Returns one of the XMLOXIDE_READER_* constants.
 */
int32_t xmloxide_reader_node_type(const xmloxide_reader *reader);

/**
 * Returns the qualified name of the current node, or NULL.
 * The returned string must be freed with xmloxide_free_string().
 */
char *xmloxide_reader_name(const xmloxide_reader *reader);

/**
 * Returns the local name of the current node (without prefix), or NULL.
 * The returned string must be freed with xmloxide_free_string().
 */
char *xmloxide_reader_local_name(const xmloxide_reader *reader);

/**
 * Returns the namespace prefix of the current node, or NULL.
 * The returned string must be freed with xmloxide_free_string().
 */
char *xmloxide_reader_prefix(const xmloxide_reader *reader);

/**
 * Returns the namespace URI of the current node, or NULL.
 * The returned string must be freed with xmloxide_free_string().
 */
char *xmloxide_reader_namespace_uri(const xmloxide_reader *reader);

/**
 * Returns the value of the current node (text, comment, attribute value),
 * or NULL for elements and end elements.
 * The returned string must be freed with xmloxide_free_string().
 */
char *xmloxide_reader_value(const xmloxide_reader *reader);

/** Returns the depth of the current node in the document tree. */
uint32_t xmloxide_reader_depth(const xmloxide_reader *reader);

/**
 * Returns 1 if the current element is self-closing (empty), 0 otherwise.
 */
int32_t xmloxide_reader_is_empty_element(const xmloxide_reader *reader);

/**
 * Returns 1 if the current node has a value, 0 otherwise.
 */
int32_t xmloxide_reader_has_value(const xmloxide_reader *reader);

/** Returns the number of attributes on the current element. */
size_t xmloxide_reader_attribute_count(const xmloxide_reader *reader);

/**
 * Returns the value of an attribute by name on the current element, or NULL.
 * The returned string must be freed with xmloxide_free_string().
 */
char *xmloxide_reader_get_attribute(const xmloxide_reader *reader,
                                     const char *name);

/**
 * Moves the reader to the first attribute of the current element.
 * Returns 1 if successful, 0 if no attributes or not on an element.
 */
int32_t xmloxide_reader_move_to_first_attribute(xmloxide_reader *reader);

/**
 * Moves the reader to the next attribute.
 * Returns 1 if successful, 0 if no more attributes.
 */
int32_t xmloxide_reader_move_to_next_attribute(xmloxide_reader *reader);

/**
 * Moves the reader back to the element from an attribute.
 * Returns 1 if moved back, 0 if not on an attribute.
 */
int32_t xmloxide_reader_move_to_element(xmloxide_reader *reader);

/**
 * Frees a reader. Passing NULL is safe and does nothing.
 */
void xmloxide_reader_free(xmloxide_reader *reader);

/* ---------- SAX2 streaming parser ---------- */

/**
 * C function pointer type for start_element events.
 *
 * Parameters:
 *   local_name  - element local name (never NULL)
 *   prefix      - namespace prefix (may be NULL)
 *   namespace   - namespace URI (may be NULL)
 *   attr_names  - array of attribute name strings
 *   attr_values - array of attribute value strings
 *   attr_count  - number of attributes
 *   user_data   - opaque pointer passed through from the handler
 */
typedef void (*xmloxide_sax_start_element_cb)(
    const char *local_name, const char *prefix, const char *namespace_uri,
    const char *const *attr_names, const char *const *attr_values,
    size_t attr_count, void *user_data);

/**
 * C function pointer type for end_element events.
 *
 * Parameters:
 *   local_name  - element local name (never NULL)
 *   prefix      - namespace prefix (may be NULL)
 *   namespace   - namespace URI (may be NULL)
 *   user_data   - opaque pointer passed through from the handler
 */
typedef void (*xmloxide_sax_end_element_cb)(const char *local_name,
                                             const char *prefix,
                                             const char *namespace_uri,
                                             void *user_data);

/**
 * C function pointer type for characters, CDATA, and comment events.
 *
 * Parameters:
 *   content   - text content (never NULL)
 *   user_data - opaque pointer passed through from the handler
 */
typedef void (*xmloxide_sax_text_cb)(const char *content, void *user_data);

/**
 * C function pointer type for processing instruction events.
 *
 * Parameters:
 *   target    - PI target (never NULL)
 *   data      - PI data (may be NULL)
 *   user_data - opaque pointer passed through from the handler
 */
typedef void (*xmloxide_sax_pi_cb)(const char *target, const char *data,
                                    void *user_data);

/**
 * SAX handler with C function pointer callbacks.
 *
 * Set any callback to NULL to ignore that event type.
 * user_data is passed through to every callback.
 */
typedef struct {
    xmloxide_sax_start_element_cb start_element;
    xmloxide_sax_end_element_cb end_element;
    xmloxide_sax_text_cb characters;
    xmloxide_sax_text_cb cdata;
    xmloxide_sax_text_cb comment;
    xmloxide_sax_pi_cb processing_instruction;
    void *user_data;
} xmloxide_sax_handler;

/**
 * Parses XML with SAX streaming, dispatching events to C function pointers.
 *
 * xml must be a valid null-terminated UTF-8 C string.
 * handler must point to a valid xmloxide_sax_handler struct.
 *
 * Returns 0 on success, -1 on error. Use xmloxide_last_error() for details.
 */
int32_t xmloxide_sax_parse(const char *xml,
                            const xmloxide_sax_handler *handler);

/* ---------- CSS selectors ---------- */

/**
 * Evaluates a CSS selector against a subtree and returns matching node IDs.
 *
 * scope is the node to search within (typically the root element).
 * selector is a null-terminated CSS selector string (e.g., "div.class > p").
 *
 * On success, sets *out_count to the number of matching nodes and returns
 * a heap-allocated array of node IDs. The caller must free the array with
 * xmloxide_free_nodeid_array(ptr, count).
 *
 * Returns NULL on failure (invalid selector or null arguments).
 */
uint32_t *xmloxide_css_select(const xmloxide_document *doc, uint32_t scope,
                               const char *selector, size_t *out_count);

/**
 * Frees a node ID array returned by xmloxide_css_select().
 * Passing NULL is safe and does nothing.
 */
void xmloxide_free_nodeid_array(uint32_t *ptr, size_t count);

/**
 * Returns the first node matching a CSS selector, or 0 if none found.
 * This is a convenience wrapper around xmloxide_css_select().
 */
uint32_t xmloxide_css_select_first(const xmloxide_document *doc, uint32_t scope,
                                    const char *selector);

/* ---------- String lifecycle ---------- */

/**
 * Frees a string previously returned by an xmloxide FFI function.
 * Passing NULL is safe and does nothing.
 */
void xmloxide_free_string(char *ptr);

#ifdef __cplusplus
}
#endif

#endif /* XMLOXIDE_H */
