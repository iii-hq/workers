/*
 * ffi_usage.c — Example of using xmloxide from C
 *
 * Build:
 *   # First build the shared library:
 *   cargo rustc --lib --release --features ffi -- --crate-type cdylib
 *
 *   # Then compile this example (adjust library path as needed):
 *   cc -o ffi_usage examples/ffi_usage.c -Iinclude \
 *      -Ltarget/release -lxmloxide -lpthread -ldl -lm
 *
 *   # On macOS, also pass: -framework Security
 *   # Run with: LD_LIBRARY_PATH=target/release ./ffi_usage
 *   #      or:  DYLD_LIBRARY_PATH=target/release ./ffi_usage
 */

#include <stdio.h>
#include <stdlib.h>
#include "xmloxide.h"

int main(void) {
    /* --- Parse an XML document --- */
    const char *xml = "<library>"
                      "  <book id=\"1\"><title>The Rust Programming Language</title></book>"
                      "  <book id=\"2\"><title>Programming Rust</title></book>"
                      "</library>";

    xmloxide_document *doc = xmloxide_parse_str(xml);
    if (!doc) {
        fprintf(stderr, "Parse error: %s\n", xmloxide_last_error());
        return 1;
    }

    /* --- Navigate the tree --- */
    uint32_t root = xmloxide_doc_root_element(doc);
    char *root_name = xmloxide_node_name(doc, root);
    printf("Root element: %s\n", root_name);
    xmloxide_free_string(root_name);

    /* Iterate children */
    uint32_t child = xmloxide_node_first_child(doc, root);
    while (child) {
        if (xmloxide_node_type(doc, child) == XMLOXIDE_NODE_ELEMENT) {
            char *name = xmloxide_node_name(doc, child);
            char *id = xmloxide_node_attribute(doc, child, "id");
            char *text = xmloxide_node_text_content(doc, child);
            printf("  <%s id=\"%s\">%s</%s>\n", name, id ? id : "", text, name);
            xmloxide_free_string(name);
            xmloxide_free_string(id);
            xmloxide_free_string(text);
        }
        child = xmloxide_node_next_sibling(doc, child);
    }

    /* --- XPath query --- */
    xmloxide_xpath_value *result = xmloxide_xpath_eval(doc, 0, "count(//book)");
    if (result) {
        printf("Book count: %.0f\n", xmloxide_xpath_result_number(result));
        xmloxide_xpath_free_result(result);
    }

    /* --- Serialize --- */
    char *output = xmloxide_serialize(doc);
    printf("Serialized: %s\n", output);
    xmloxide_free_string(output);

    /* --- Pretty-print --- */
    char *pretty = xmloxide_serialize_pretty(doc);
    printf("Pretty:\n%s\n", pretty);
    xmloxide_free_string(pretty);

    /* --- Mutate the tree --- */
    uint32_t new_book = xmloxide_create_element(doc, "book");
    xmloxide_set_attribute(doc, new_book, "id", "3");
    uint32_t title = xmloxide_create_element(doc, "title");
    uint32_t title_text = xmloxide_create_text(doc, "Zero To Production");
    xmloxide_append_child(doc, title, title_text);
    xmloxide_append_child(doc, new_book, title);
    xmloxide_append_child(doc, root, new_book);

    char *after = xmloxide_serialize(doc);
    printf("After mutation: %s\n", after);
    xmloxide_free_string(after);

    xmloxide_free_doc(doc);

    printf("Done.\n");
    return 0;
}
