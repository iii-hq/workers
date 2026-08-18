//! XML Deserializer backed by the xmloxide DOM tree.

use serde::de::{self, DeserializeSeed, MapAccess, SeqAccess, Visitor};
use serde::Deserialize;

use crate::tree::{Document, NodeId, NodeKind};

use super::Error;

/// Deserializes a Rust value from an XML string.
///
/// The root element maps to the top-level struct.
///
/// # Errors
///
/// Returns an error if parsing or deserialization fails.
pub fn from_str<'de, T: Deserialize<'de>>(xml: &str) -> Result<T, Error> {
    let doc = Document::parse_str(xml)?;
    let root = doc
        .root_element()
        .ok_or_else(|| Error::Message("no root element".to_string()))?;
    let de = Deserializer::new(&doc, root);
    T::deserialize(de)
}

/// An XML element deserializer.
struct Deserializer<'a> {
    doc: &'a Document,
    node: NodeId,
}

impl<'a> Deserializer<'a> {
    fn new(doc: &'a Document, node: NodeId) -> Self {
        Self { doc, node }
    }

    /// Collects the text content of this element (concatenating child text nodes).
    fn text_content(&self) -> String {
        self.doc.text_content(self.node)
    }
}

impl<'de> de::Deserializer<'de> for Deserializer<'_> {
    type Error = Error;

    fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        self.deserialize_map(visitor)
    }

    fn deserialize_bool<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        let text = self.text_content();
        match text.as_str() {
            "true" | "1" => visitor.visit_bool(true),
            "false" | "0" => visitor.visit_bool(false),
            _ => Err(Error::Message(format!("invalid bool: {text}"))),
        }
    }

    fn deserialize_i8<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        let text = self.text_content();
        visitor.visit_i8(
            text.parse()
                .map_err(|_| Error::Message(format!("invalid i8: {text}")))?,
        )
    }

    fn deserialize_i16<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        let text = self.text_content();
        visitor.visit_i16(
            text.parse()
                .map_err(|_| Error::Message(format!("invalid i16: {text}")))?,
        )
    }

    fn deserialize_i32<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        let text = self.text_content();
        visitor.visit_i32(
            text.parse()
                .map_err(|_| Error::Message(format!("invalid i32: {text}")))?,
        )
    }

    fn deserialize_i64<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        let text = self.text_content();
        visitor.visit_i64(
            text.parse()
                .map_err(|_| Error::Message(format!("invalid i64: {text}")))?,
        )
    }

    fn deserialize_u8<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        let text = self.text_content();
        visitor.visit_u8(
            text.parse()
                .map_err(|_| Error::Message(format!("invalid u8: {text}")))?,
        )
    }

    fn deserialize_u16<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        let text = self.text_content();
        visitor.visit_u16(
            text.parse()
                .map_err(|_| Error::Message(format!("invalid u16: {text}")))?,
        )
    }

    fn deserialize_u32<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        let text = self.text_content();
        visitor.visit_u32(
            text.parse()
                .map_err(|_| Error::Message(format!("invalid u32: {text}")))?,
        )
    }

    fn deserialize_u64<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        let text = self.text_content();
        visitor.visit_u64(
            text.parse()
                .map_err(|_| Error::Message(format!("invalid u64: {text}")))?,
        )
    }

    fn deserialize_f32<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        let text = self.text_content();
        visitor.visit_f32(
            text.parse()
                .map_err(|_| Error::Message(format!("invalid f32: {text}")))?,
        )
    }

    fn deserialize_f64<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        let text = self.text_content();
        visitor.visit_f64(
            text.parse()
                .map_err(|_| Error::Message(format!("invalid f64: {text}")))?,
        )
    }

    fn deserialize_char<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        let text = self.text_content();
        let mut chars = text.chars();
        let c = chars
            .next()
            .ok_or_else(|| Error::Message("empty char".to_string()))?;
        if chars.next().is_some() {
            return Err(Error::Message(format!("expected single char, got: {text}")));
        }
        visitor.visit_char(c)
    }

    fn deserialize_str<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        visitor.visit_string(self.text_content())
    }

    fn deserialize_string<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        visitor.visit_string(self.text_content())
    }

    fn deserialize_bytes<V: Visitor<'de>>(self, _visitor: V) -> Result<V::Value, Self::Error> {
        Err(Error::Message("bytes not supported in XML".to_string()))
    }

    fn deserialize_byte_buf<V: Visitor<'de>>(self, _visitor: V) -> Result<V::Value, Self::Error> {
        Err(Error::Message("byte_buf not supported in XML".to_string()))
    }

    fn deserialize_option<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        visitor.visit_some(self)
    }

    fn deserialize_unit<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        visitor.visit_unit()
    }

    fn deserialize_unit_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        visitor: V,
    ) -> Result<V::Value, Self::Error> {
        visitor.visit_unit()
    }

    fn deserialize_newtype_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        visitor: V,
    ) -> Result<V::Value, Self::Error> {
        visitor.visit_newtype_struct(self)
    }

    fn deserialize_seq<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        let children: Vec<NodeId> = self
            .doc
            .children(self.node)
            .filter(|&c| matches!(self.doc.node(c).kind, NodeKind::Element { .. }))
            .collect();
        visitor.visit_seq(SeqDeserializer {
            doc: self.doc,
            children,
            index: 0,
        })
    }

    fn deserialize_tuple<V: Visitor<'de>>(
        self,
        _len: usize,
        visitor: V,
    ) -> Result<V::Value, Self::Error> {
        self.deserialize_seq(visitor)
    }

    fn deserialize_tuple_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        _len: usize,
        visitor: V,
    ) -> Result<V::Value, Self::Error> {
        self.deserialize_seq(visitor)
    }

    fn deserialize_map<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        visitor.visit_map(ElementMapAccess::new(self.doc, self.node))
    }

    fn deserialize_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        _fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Self::Error> {
        self.deserialize_map(visitor)
    }

    fn deserialize_enum<V: Visitor<'de>>(
        self,
        _name: &'static str,
        _variants: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Self::Error> {
        let text = self.text_content();
        visitor.visit_enum(StringIntoDeserializer(text))
    }

    fn deserialize_identifier<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        self.deserialize_string(visitor)
    }

    fn deserialize_ignored_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        visitor.visit_unit()
    }
}

/// Map access over an XML element: yields attributes (as `$attr:name`) and child elements.
struct ElementMapAccess<'a> {
    doc: &'a Document,
    node: NodeId,
    attrs: Vec<(String, String)>,
    children: Vec<(String, Vec<NodeId>)>,
    index: usize,
    keys: Vec<String>,
}

impl<'a> ElementMapAccess<'a> {
    fn new(doc: &'a Document, node: NodeId) -> Self {
        let attrs: Vec<(String, String)> = doc
            .attributes(node)
            .iter()
            .map(|a| (format!("$attr:{}", a.name), a.value.clone()))
            .collect();

        let mut child_map: Vec<(String, Vec<NodeId>)> = Vec::new();
        for child_id in doc.children(node) {
            if let NodeKind::Element { ref name, .. } = doc.node(child_id).kind {
                if let Some(entry) = child_map.iter_mut().find(|(n, _)| n == name) {
                    entry.1.push(child_id);
                } else {
                    child_map.push((name.clone(), vec![child_id]));
                }
            }
        }

        let has_text = doc
            .children(node)
            .any(|c| matches!(doc.node(c).kind, NodeKind::Text { .. }));

        let mut keys: Vec<String> = attrs.iter().map(|(k, _)| k.clone()).collect();
        if has_text {
            keys.push("$text".to_string());
        }
        for (name, _) in &child_map {
            keys.push(name.clone());
        }

        Self {
            doc,
            node,
            attrs,
            children: child_map,
            index: 0,
            keys,
        }
    }
}

impl<'de> MapAccess<'de> for ElementMapAccess<'_> {
    type Error = Error;

    fn next_key_seed<K: DeserializeSeed<'de>>(
        &mut self,
        seed: K,
    ) -> Result<Option<K::Value>, Self::Error> {
        if self.index >= self.keys.len() {
            return Ok(None);
        }
        let key = &self.keys[self.index];
        seed.deserialize(de::value::StrDeserializer::new(key))
            .map(Some)
    }

    fn next_value_seed<V: DeserializeSeed<'de>>(
        &mut self,
        seed: V,
    ) -> Result<V::Value, Self::Error> {
        let key = &self.keys[self.index];
        self.index += 1;

        if let Some(attr) = self.attrs.iter().find(|(k, _)| k == key) {
            return seed.deserialize(de::value::StringDeserializer::new(attr.1.clone()));
        }

        if key == "$text" {
            let text = self.doc.text_content(self.node);
            return seed.deserialize(de::value::StringDeserializer::new(text));
        }

        if let Some(entry) = self.children.iter().find(|(n, _)| n == key) {
            let nodes = &entry.1;
            if nodes.len() == 1 {
                return seed.deserialize(Deserializer::new(self.doc, nodes[0]));
            }
            return seed.deserialize(SeqNodeDeserializer {
                doc: self.doc,
                nodes: nodes.clone(),
            });
        }

        Err(Error::Message(format!("unexpected key: {key}")))
    }
}

/// Deserializer that presents multiple nodes as a sequence.
struct SeqNodeDeserializer<'a> {
    doc: &'a Document,
    nodes: Vec<NodeId>,
}

impl<'de> de::Deserializer<'de> for SeqNodeDeserializer<'_> {
    type Error = Error;

    fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        self.deserialize_seq(visitor)
    }

    fn deserialize_seq<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        visitor.visit_seq(SeqDeserializer {
            doc: self.doc,
            children: self.nodes,
            index: 0,
        })
    }

    serde::forward_to_deserialize_any! {
        bool i8 i16 i32 i64 u8 u16 u32 u64 f32 f64 char str string bytes
        byte_buf option unit unit_struct newtype_struct tuple tuple_struct
        map struct enum identifier ignored_any
    }
}

/// Sequential access over a list of child nodes.
struct SeqDeserializer<'a> {
    doc: &'a Document,
    children: Vec<NodeId>,
    index: usize,
}

impl<'de> SeqAccess<'de> for SeqDeserializer<'_> {
    type Error = Error;

    fn next_element_seed<T: DeserializeSeed<'de>>(
        &mut self,
        seed: T,
    ) -> Result<Option<T::Value>, Self::Error> {
        if self.index >= self.children.len() {
            return Ok(None);
        }
        let node = self.children[self.index];
        self.index += 1;
        seed.deserialize(Deserializer::new(self.doc, node))
            .map(Some)
    }
}

/// Helper: wraps a `String` as a serde enum deserializer for simple string enums.
struct StringEnumDeserializer(String);

impl<'de> de::Deserializer<'de> for StringEnumDeserializer {
    type Error = Error;

    fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        visitor.visit_string(self.0)
    }

    serde::forward_to_deserialize_any! {
        bool i8 i16 i32 i64 u8 u16 u32 u64 f32 f64 char str string bytes
        byte_buf option unit unit_struct newtype_struct seq tuple tuple_struct
        map struct enum identifier ignored_any
    }
}

/// Newtype for using `String` as an `EnumAccess` for unit variants.
struct StringIntoDeserializer(String);

impl<'de> de::EnumAccess<'de> for StringIntoDeserializer {
    type Error = Error;
    type Variant = UnitVariantAccess;

    fn variant_seed<V: DeserializeSeed<'de>>(
        self,
        seed: V,
    ) -> Result<(V::Value, Self::Variant), Self::Error> {
        let val = seed.deserialize(StringEnumDeserializer(self.0))?;
        Ok((val, UnitVariantAccess))
    }
}

/// Unit variant access (no data associated with the variant).
struct UnitVariantAccess;

impl<'de> de::VariantAccess<'de> for UnitVariantAccess {
    type Error = Error;

    fn unit_variant(self) -> Result<(), Self::Error> {
        Ok(())
    }

    fn newtype_variant_seed<T: DeserializeSeed<'de>>(
        self,
        _seed: T,
    ) -> Result<T::Value, Self::Error> {
        Err(Error::Message("newtype variant not supported".to_string()))
    }

    fn tuple_variant<V: Visitor<'de>>(
        self,
        _len: usize,
        _visitor: V,
    ) -> Result<V::Value, Self::Error> {
        Err(Error::Message("tuple variant not supported".to_string()))
    }

    fn struct_variant<V: Visitor<'de>>(
        self,
        _fields: &'static [&'static str],
        _visitor: V,
    ) -> Result<V::Value, Self::Error> {
        Err(Error::Message("struct variant not supported".to_string()))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[test]
    fn test_de_simple_struct() {
        #[derive(Debug, Deserialize, PartialEq)]
        struct Root {
            name: String,
            value: String,
        }
        let xml = "<Root><name>hello</name><value>world</value></Root>";
        let r: Root = from_str(xml).unwrap();
        assert_eq!(r.name, "hello");
        assert_eq!(r.value, "world");
    }

    #[test]
    fn test_de_attributes() {
        #[derive(Debug, Deserialize, PartialEq)]
        struct Item {
            #[serde(rename = "$attr:id")]
            id: String,
            #[serde(rename = "$attr:class")]
            class: String,
        }
        let xml = r#"<Item id="1" class="foo"/>"#;
        let item: Item = from_str(xml).unwrap();
        assert_eq!(item.id, "1");
        assert_eq!(item.class, "foo");
    }

    #[test]
    fn test_de_text_content() {
        #[derive(Debug, Deserialize, PartialEq)]
        struct Msg {
            #[serde(rename = "$text")]
            text: String,
        }
        let xml = "<Msg>Hello World</Msg>";
        let msg: Msg = from_str(xml).unwrap();
        assert_eq!(msg.text, "Hello World");
    }

    #[test]
    fn test_de_nested() {
        #[derive(Debug, Deserialize, PartialEq)]
        struct Inner {
            #[serde(rename = "$text")]
            text: String,
        }
        #[derive(Debug, Deserialize, PartialEq)]
        struct Outer {
            inner: Inner,
        }
        let xml = "<Outer><inner>data</inner></Outer>";
        let o: Outer = from_str(xml).unwrap();
        assert_eq!(o.inner.text, "data");
    }

    #[test]
    fn test_de_sequence() {
        #[derive(Debug, Deserialize, PartialEq)]
        struct Item {
            #[serde(rename = "$text")]
            text: String,
        }
        #[derive(Debug, Deserialize, PartialEq)]
        struct List {
            item: Vec<Item>,
        }
        let xml = "<List><item>A</item><item>B</item><item>C</item></List>";
        let list: List = from_str(xml).unwrap();
        assert_eq!(list.item.len(), 3);
        assert_eq!(list.item[0].text, "A");
        assert_eq!(list.item[2].text, "C");
    }

    #[test]
    fn test_de_numeric() {
        #[derive(Debug, Deserialize, PartialEq)]
        struct Data {
            count: u32,
            ratio: f64,
        }
        let xml = "<Data><count>42</count><ratio>2.72</ratio></Data>";
        let d: Data = from_str(xml).unwrap();
        assert_eq!(d.count, 42);
        assert!((d.ratio - 2.72).abs() < f64::EPSILON);
    }

    #[test]
    fn test_de_bool() {
        #[derive(Debug, Deserialize, PartialEq)]
        struct Flags {
            active: bool,
            visible: bool,
        }
        let xml = "<Flags><active>true</active><visible>false</visible></Flags>";
        let f: Flags = from_str(xml).unwrap();
        assert!(f.active);
        assert!(!f.visible);
    }

    #[test]
    fn test_de_option_present() {
        #[derive(Debug, Deserialize, PartialEq)]
        struct Data {
            #[serde(default)]
            value: Option<String>,
        }
        let xml = "<Data><value>yes</value></Data>";
        let d: Data = from_str(xml).unwrap();
        assert_eq!(d.value, Some("yes".to_string()));
    }

    #[test]
    fn test_de_mixed_attrs_and_children() {
        #[derive(Debug, Deserialize, PartialEq)]
        struct Node {
            #[serde(rename = "$attr:type")]
            node_type: String,
            child: String,
        }
        let xml = r#"<Node type="special"><child>data</child></Node>"#;
        let n: Node = from_str(xml).unwrap();
        assert_eq!(n.node_type, "special");
        assert_eq!(n.child, "data");
    }

    #[test]
    fn test_de_renamed_root() {
        #[derive(Debug, Deserialize, PartialEq)]
        #[serde(rename = "book")]
        struct Book {
            title: String,
        }
        let xml = "<book><title>Rust in Action</title></book>";
        let b: Book = from_str(xml).unwrap();
        assert_eq!(b.title, "Rust in Action");
    }
}
