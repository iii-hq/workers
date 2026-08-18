/*
 * libxml2_compat.h — libxml2-like API adaptor for xmloxide
 *
 * This header provides a thin compatibility layer that maps common libxml2
 * function names and types to xmloxide's C FFI. It covers the most frequently
 * used libxml2 APIs (parsing, tree navigation, serialization, XPath) to ease
 * migration from libxml2 to xmloxide.
 *
 * Usage:
 *   #include "libxml2_compat.h"
 *   // Use familiar libxml2 names — they delegate to xmloxide
 *
 * Limitations:
 *   - Node pointers are NOT dereferenceable structs. You cannot write
 *     node->name or node->children. Use the accessor functions instead.
 *   - No global state: xmlInitParser() and xmlCleanupParser() are no-ops.
 *   - No custom error handlers (xmlSetGenericErrorFunc is a no-op).
 *   - Only covers commonly-used APIs. See xmloxide.h for the full API.
 *
 * Requires: xmloxide.h (include it first or let this header include it).
 */

#ifndef LIBXML2_COMPAT_H
#define LIBXML2_COMPAT_H

#include "xmloxide.h"

#ifdef __cplusplus
extern "C" {
#endif

/* ========================================================================
 * Type aliases
 * ======================================================================== */

/** Opaque document type (replaces libxml2's xmlDoc / xmlDocPtr). */
typedef xmloxide_document xmlDoc;
typedef xmloxide_document *xmlDocPtr;

/**
 * Node handle — NOT a dereferenceable pointer like libxml2's xmlNode.
 *
 * In xmloxide, nodes are identified by a (document, node_id) pair.
 * This struct wraps both so you can pass "node pointers" around.
 * Access node properties via xmlNodeGetName(), xmlNodeGetContent(), etc.
 */
typedef struct {
    xmloxide_document *doc;
    uint32_t id;
} xmlNode;
typedef xmlNode *xmlNodePtr;

/** XPath result type. */
typedef xmloxide_xpath_value xmlXPathObject;
typedef xmloxide_xpath_value *xmlXPathObjectPtr;

/* ========================================================================
 * Global lifecycle — no-ops (xmloxide has no global state)
 * ======================================================================== */

/** No-op. xmloxide requires no global initialization. */
static inline void xmlInitParser(void) {}

/** No-op. xmloxide requires no global cleanup. */
static inline void xmlCleanupParser(void) {}

/** No-op. xmloxide has no global memory tracking. */
static inline void xmlMemoryDump(void) {}

/* ========================================================================
 * Parsing
 * ======================================================================== */

/** Parse a null-terminated XML string. Returns NULL on failure. */
static inline xmlDocPtr xmlParseDoc(const char *input) {
    return xmloxide_parse_str(input);
}

/** Parse a buffer of `size` bytes as XML. Returns NULL on failure. */
static inline xmlDocPtr xmlReadMemory(const char *buffer, int size,
                                       const char *url, const char *encoding,
                                       int options) {
    (void)url; (void)encoding; (void)options;
    return xmloxide_parse_bytes((const uint8_t *)buffer, (size_t)size);
}

/** Parse an XML file. Returns NULL on failure. */
static inline xmlDocPtr xmlReadFile(const char *filename, const char *encoding,
                                     int options) {
    (void)encoding; (void)options;
    return xmloxide_parse_file(filename);
}

/** Parse an HTML string. Returns NULL on failure. */
static inline xmlDocPtr htmlReadMemory(const char *buffer, int size,
                                        const char *url, const char *encoding,
                                        int options) {
    (void)size; (void)url; (void)encoding; (void)options;
    return xmloxide_parse_html(buffer);
}

/** Parse an HTML5 string. Returns NULL on failure. */
static inline xmlDocPtr htmlReadMemory5(const char *buffer, int size,
                                         const char *url, const char *encoding,
                                         int options) {
    (void)size; (void)url; (void)encoding; (void)options;
    return xmloxide_parse_html5(buffer);
}

/** Free a document. */
static inline void xmlFreeDoc(xmlDocPtr doc) {
    xmloxide_free_doc(doc);
}

/* ========================================================================
 * Tree navigation — returns heap-allocated xmlNode (caller must free)
 * ======================================================================== */

/** Allocate an xmlNode handle. Caller must free with xmlFreeNode(). */
static inline xmlNodePtr xmloxide_compat_make_node(xmlDocPtr doc, uint32_t id) {
    if (id == 0) return NULL;
    xmlNodePtr node = (xmlNodePtr)malloc(sizeof(xmlNode));
    if (node) {
        node->doc = doc;
        node->id = id;
    }
    return node;
}

/** Free an xmlNode handle. Does NOT remove the node from the tree. */
static inline void xmlFreeNode(xmlNodePtr node) {
    free(node);
}

/** Get the root element. Caller must free the returned node with xmlFreeNode(). */
static inline xmlNodePtr xmlDocGetRootElement(xmlDocPtr doc) {
    return xmloxide_compat_make_node(doc, xmloxide_doc_root_element(doc));
}

/** Get the parent node. Caller must free with xmlFreeNode(). */
static inline xmlNodePtr xmlNodeGetParent(xmlNodePtr node) {
    if (!node) return NULL;
    return xmloxide_compat_make_node(node->doc,
        xmloxide_node_parent(node->doc, node->id));
}

/** Get the first child. Caller must free with xmlFreeNode(). */
static inline xmlNodePtr xmlNodeGetChildren(xmlNodePtr node) {
    if (!node) return NULL;
    return xmloxide_compat_make_node(node->doc,
        xmloxide_node_first_child(node->doc, node->id));
}

/** Get the next sibling. Caller must free with xmlFreeNode(). */
static inline xmlNodePtr xmlNodeGetNext(xmlNodePtr node) {
    if (!node) return NULL;
    return xmloxide_compat_make_node(node->doc,
        xmloxide_node_next_sibling(node->doc, node->id));
}

/** Get the previous sibling. Caller must free with xmlFreeNode(). */
static inline xmlNodePtr xmlNodeGetPrev(xmlNodePtr node) {
    if (!node) return NULL;
    return xmloxide_compat_make_node(node->doc,
        xmloxide_node_prev_sibling(node->doc, node->id));
}

/* ========================================================================
 * Node inspection
 * ======================================================================== */

/** Get the node type (returns XMLOXIDE_NODE_* constants). */
static inline int xmlNodeGetType(xmlNodePtr node) {
    if (!node) return -1;
    return xmloxide_node_type(node->doc, node->id);
}

/**
 * Get the node name. Returns a caller-owned string that must be freed
 * with xmlFree().
 */
static inline char *xmlNodeGetName(xmlNodePtr node) {
    if (!node) return NULL;
    return xmloxide_node_name(node->doc, node->id);
}

/**
 * Get the concatenated text content. Returns a caller-owned string
 * that must be freed with xmlFree().
 */
static inline char *xmlNodeGetContent(xmlNodePtr node) {
    if (!node) return NULL;
    return xmloxide_node_text_content(node->doc, node->id);
}

/**
 * Get an attribute value by name. Returns a caller-owned string
 * that must be freed with xmlFree().
 */
static inline char *xmlGetProp(xmlNodePtr node, const char *name) {
    if (!node) return NULL;
    return xmloxide_node_attribute(node->doc, node->id, name);
}

/**
 * Set an attribute value. Returns 1 on success, 0 on failure.
 */
static inline int xmlSetProp(xmlNodePtr node, const char *name,
                              const char *value) {
    if (!node) return 0;
    return xmloxide_set_attribute(node->doc, node->id, name, value);
}

/**
 * Remove an attribute by name. Returns 1 if removed, 0 otherwise.
 */
static inline int xmlUnsetProp(xmlNodePtr node, const char *name) {
    if (!node) return 0;
    return xmloxide_remove_attribute(node->doc, node->id, name);
}

/* ========================================================================
 * Serialization
 * ======================================================================== */

/**
 * Serialize a document to XML. The caller must free the result with xmlFree().
 * `mem` receives the string pointer, `size` receives the length.
 */
static inline void xmlDocDumpMemory(xmlDocPtr doc, char **mem, int *size) {
    if (!doc || !mem) return;
    char *s = xmloxide_serialize(doc);
    *mem = s;
    if (size) *size = s ? (int)strlen(s) : 0;
}

/**
 * Serialize a document to pretty-printed XML. The caller must free the
 * result with xmlFree(). `mem` receives the string pointer, `size` the length.
 */
static inline void xmlDocDumpFormatMemory(xmlDocPtr doc, char **mem,
                                           int *size, int format) {
    (void)format;
    if (!doc || !mem) return;
    char *s = xmloxide_serialize_pretty(doc);
    *mem = s;
    if (size) *size = s ? (int)strlen(s) : 0;
}

/* ========================================================================
 * String lifecycle
 * ======================================================================== */

/**
 * Free a string returned by xmloxide (replaces libxml2's xmlFree for strings).
 */
static inline void xmlFree(void *ptr) {
    xmloxide_free_string((char *)ptr);
}

/* ========================================================================
 * XPath
 * ======================================================================== */

/**
 * Evaluate an XPath expression. Returns NULL on failure.
 * The result must be freed with xmlXPathFreeObject().
 */
static inline xmlXPathObjectPtr xmlXPathEval(const char *expr,
                                              xmlNodePtr context) {
    if (!context) return NULL;
    return xmloxide_xpath_eval(context->doc, context->id, expr);
}

/** Free an XPath result. */
static inline void xmlXPathFreeObject(xmlXPathObjectPtr obj) {
    xmloxide_xpath_free_result(obj);
}

/** Get the number of nodes in an XPath nodeset result. */
static inline int xmlXPathNodeSetGetLength(xmlXPathObjectPtr obj) {
    return (int)xmloxide_xpath_nodeset_count(obj);
}

/**
 * Get a node from an XPath nodeset by index.
 * NOTE: Unlike libxml2, this requires the original document pointer.
 * The returned node must be freed with xmlFreeNode().
 */
static inline xmlNodePtr xmlXPathNodeSetItem(xmlXPathObjectPtr obj,
                                              int index,
                                              xmlDocPtr doc) {
    uint32_t id = xmloxide_xpath_nodeset_item(obj, (size_t)index);
    return xmloxide_compat_make_node(doc, id);
}

/* ========================================================================
 * Error handling
 * ======================================================================== */

/** Get the last error message. Library-owned — do NOT free. */
static inline const char *xmlGetLastError(void) {
    return xmloxide_last_error();
}

/** No-op. xmloxide uses thread-local error storage, not callbacks. */
static inline void xmlSetGenericErrorFunc(void *ctx,
                                           void (*handler)(void *, const char *, ...)) {
    (void)ctx; (void)handler;
}

#ifdef __cplusplus
}
#endif

#endif /* LIBXML2_COMPAT_H */
