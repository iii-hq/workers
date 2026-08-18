//! XML Serializer that produces XML strings from Rust types via serde.

use serde::ser::{self, Serialize};

use super::Error;

/// Serializes a Rust value to an XML string.
///
/// The struct's name (or `#[serde(rename = "...")]`) becomes the root element.
/// Fields prefixed with `$attr:` become attributes. A field named `$text`
/// becomes the element's text content.
///
/// # Errors
///
/// Returns an error if serialization fails.
pub fn to_string<T: Serialize>(value: &T) -> Result<String, Error> {
    let mut output = String::new();
    let serializer = XmlSerializer {
        output: &mut output,
    };
    value.serialize(serializer)?;
    Ok(output)
}

struct XmlSerializer<'a> {
    output: &'a mut String,
}

impl<'a> ser::Serializer for XmlSerializer<'a> {
    type Ok = ();
    type Error = Error;
    type SerializeSeq = SeqSerializer<'a>;
    type SerializeTuple = SeqSerializer<'a>;
    type SerializeTupleStruct = SeqSerializer<'a>;
    type SerializeTupleVariant = SeqSerializer<'a>;
    type SerializeMap = MapSerializer<'a>;
    type SerializeStruct = StructSerializer<'a>;
    type SerializeStructVariant = StructSerializer<'a>;

    fn serialize_bool(self, v: bool) -> Result<Self::Ok, Self::Error> {
        self.output.push_str(if v { "true" } else { "false" });
        Ok(())
    }

    fn serialize_i8(self, v: i8) -> Result<Self::Ok, Self::Error> {
        self.output.push_str(&v.to_string());
        Ok(())
    }

    fn serialize_i16(self, v: i16) -> Result<Self::Ok, Self::Error> {
        self.output.push_str(&v.to_string());
        Ok(())
    }

    fn serialize_i32(self, v: i32) -> Result<Self::Ok, Self::Error> {
        self.output.push_str(&v.to_string());
        Ok(())
    }

    fn serialize_i64(self, v: i64) -> Result<Self::Ok, Self::Error> {
        self.output.push_str(&v.to_string());
        Ok(())
    }

    fn serialize_u8(self, v: u8) -> Result<Self::Ok, Self::Error> {
        self.output.push_str(&v.to_string());
        Ok(())
    }

    fn serialize_u16(self, v: u16) -> Result<Self::Ok, Self::Error> {
        self.output.push_str(&v.to_string());
        Ok(())
    }

    fn serialize_u32(self, v: u32) -> Result<Self::Ok, Self::Error> {
        self.output.push_str(&v.to_string());
        Ok(())
    }

    fn serialize_u64(self, v: u64) -> Result<Self::Ok, Self::Error> {
        self.output.push_str(&v.to_string());
        Ok(())
    }

    fn serialize_f32(self, v: f32) -> Result<Self::Ok, Self::Error> {
        self.output.push_str(&v.to_string());
        Ok(())
    }

    fn serialize_f64(self, v: f64) -> Result<Self::Ok, Self::Error> {
        self.output.push_str(&v.to_string());
        Ok(())
    }

    fn serialize_char(self, v: char) -> Result<Self::Ok, Self::Error> {
        escape_xml_to(self.output, &v.to_string());
        Ok(())
    }

    fn serialize_str(self, v: &str) -> Result<Self::Ok, Self::Error> {
        escape_xml_to(self.output, v);
        Ok(())
    }

    fn serialize_bytes(self, _v: &[u8]) -> Result<Self::Ok, Self::Error> {
        Err(Error::Message("bytes not supported in XML".to_string()))
    }

    fn serialize_none(self) -> Result<Self::Ok, Self::Error> {
        Ok(())
    }

    fn serialize_some<T: ?Sized + Serialize>(self, value: &T) -> Result<Self::Ok, Self::Error> {
        value.serialize(self)
    }

    fn serialize_unit(self) -> Result<Self::Ok, Self::Error> {
        Ok(())
    }

    fn serialize_unit_struct(self, _name: &'static str) -> Result<Self::Ok, Self::Error> {
        Ok(())
    }

    fn serialize_unit_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        variant: &'static str,
    ) -> Result<Self::Ok, Self::Error> {
        self.output.push_str(variant);
        Ok(())
    }

    fn serialize_newtype_struct<T: ?Sized + Serialize>(
        self,
        _name: &'static str,
        value: &T,
    ) -> Result<Self::Ok, Self::Error> {
        value.serialize(self)
    }

    fn serialize_newtype_variant<T: ?Sized + Serialize>(
        self,
        _name: &'static str,
        _variant_index: u32,
        variant: &'static str,
        value: &T,
    ) -> Result<Self::Ok, Self::Error> {
        self.output.push('<');
        self.output.push_str(variant);
        self.output.push('>');
        value.serialize(XmlSerializer {
            output: self.output,
        })?;
        self.output.push_str("</");
        self.output.push_str(variant);
        self.output.push('>');
        Ok(())
    }

    fn serialize_seq(self, _len: Option<usize>) -> Result<Self::SerializeSeq, Self::Error> {
        Ok(SeqSerializer {
            output: self.output,
        })
    }

    fn serialize_tuple(self, _len: usize) -> Result<Self::SerializeTuple, Self::Error> {
        Ok(SeqSerializer {
            output: self.output,
        })
    }

    fn serialize_tuple_struct(
        self,
        _name: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeTupleStruct, Self::Error> {
        Ok(SeqSerializer {
            output: self.output,
        })
    }

    fn serialize_tuple_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        _variant: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeTupleVariant, Self::Error> {
        Ok(SeqSerializer {
            output: self.output,
        })
    }

    fn serialize_map(self, _len: Option<usize>) -> Result<Self::SerializeMap, Self::Error> {
        Ok(MapSerializer {
            output: self.output,
            current_key: None,
        })
    }

    fn serialize_struct(
        self,
        name: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeStruct, Self::Error> {
        Ok(StructSerializer {
            output: self.output,
            tag: name.to_string(),
            attrs: String::new(),
            body: String::new(),
        })
    }

    fn serialize_struct_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        variant: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeStructVariant, Self::Error> {
        Ok(StructSerializer {
            output: self.output,
            tag: variant.to_string(),
            attrs: String::new(),
            body: String::new(),
        })
    }
}

/// Serializer for struct fields — collects attrs and child elements, then emits XML.
struct StructSerializer<'a> {
    output: &'a mut String,
    tag: String,
    attrs: String,
    body: String,
}

impl ser::SerializeStruct for StructSerializer<'_> {
    type Ok = ();
    type Error = Error;

    fn serialize_field<T: ?Sized + Serialize>(
        &mut self,
        key: &'static str,
        value: &T,
    ) -> Result<(), Self::Error> {
        if let Some(attr_name) = key.strip_prefix("$attr:") {
            let mut val_str = String::new();
            value.serialize(XmlSerializer {
                output: &mut val_str,
            })?;
            self.attrs.push(' ');
            self.attrs.push_str(attr_name);
            self.attrs.push_str("=\"");
            escape_xml_attr_to(&mut self.attrs, &val_str);
            self.attrs.push('"');
        } else if key == "$text" {
            value.serialize(XmlSerializer {
                output: &mut self.body,
            })?;
        } else {
            let mut child_buf = String::new();
            value.serialize(FieldSerializer {
                output: &mut child_buf,
                tag: key,
            })?;
            self.body.push_str(&child_buf);
        }
        Ok(())
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        self.output.push('<');
        self.output.push_str(&self.tag);
        self.output.push_str(&self.attrs);
        if self.body.is_empty() {
            self.output.push_str("/>");
        } else {
            self.output.push('>');
            self.output.push_str(&self.body);
            self.output.push_str("</");
            self.output.push_str(&self.tag);
            self.output.push('>');
        }
        Ok(())
    }
}

impl ser::SerializeStructVariant for StructSerializer<'_> {
    type Ok = ();
    type Error = Error;

    fn serialize_field<T: ?Sized + Serialize>(
        &mut self,
        key: &'static str,
        value: &T,
    ) -> Result<(), Self::Error> {
        ser::SerializeStruct::serialize_field(self, key, value)
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        ser::SerializeStruct::end(self)
    }
}

/// Serializer for a struct field that wraps scalar values in `<tag>...</tag>`.
/// For sequences (`Vec`), each element gets its own `<tag>` wrapper.
struct FieldSerializer<'a> {
    output: &'a mut String,
    tag: &'a str,
}

impl FieldSerializer<'_> {
    fn wrap_scalar(self, value: &str) {
        self.output.push('<');
        self.output.push_str(self.tag);
        self.output.push('>');
        self.output.push_str(value);
        self.output.push_str("</");
        self.output.push_str(self.tag);
        self.output.push('>');
    }

    fn wrap_scalar_escaped(self, value: &str) {
        self.output.push('<');
        self.output.push_str(self.tag);
        self.output.push('>');
        escape_xml_to(self.output, value);
        self.output.push_str("</");
        self.output.push_str(self.tag);
        self.output.push('>');
    }
}

impl<'a> ser::Serializer for FieldSerializer<'a> {
    type Ok = ();
    type Error = Error;
    type SerializeSeq = SeqFieldSerializer<'a>;
    type SerializeTuple = SeqFieldSerializer<'a>;
    type SerializeTupleStruct = SeqFieldSerializer<'a>;
    type SerializeTupleVariant = SeqFieldSerializer<'a>;
    type SerializeMap = MapSerializer<'a>;
    type SerializeStruct = StructSerializer<'a>;
    type SerializeStructVariant = StructSerializer<'a>;

    fn serialize_bool(self, v: bool) -> Result<Self::Ok, Self::Error> {
        self.output.push('<');
        self.output.push_str(self.tag);
        self.output.push('>');
        self.output.push_str(if v { "true" } else { "false" });
        self.output.push_str("</");
        self.output.push_str(self.tag);
        self.output.push('>');
        Ok(())
    }

    fn serialize_i8(self, v: i8) -> Result<Self::Ok, Self::Error> {
        self.wrap_scalar(&v.to_string());
        Ok(())
    }
    fn serialize_i16(self, v: i16) -> Result<Self::Ok, Self::Error> {
        self.wrap_scalar(&v.to_string());
        Ok(())
    }
    fn serialize_i32(self, v: i32) -> Result<Self::Ok, Self::Error> {
        self.wrap_scalar(&v.to_string());
        Ok(())
    }
    fn serialize_i64(self, v: i64) -> Result<Self::Ok, Self::Error> {
        self.wrap_scalar(&v.to_string());
        Ok(())
    }
    fn serialize_u8(self, v: u8) -> Result<Self::Ok, Self::Error> {
        self.wrap_scalar(&v.to_string());
        Ok(())
    }
    fn serialize_u16(self, v: u16) -> Result<Self::Ok, Self::Error> {
        self.wrap_scalar(&v.to_string());
        Ok(())
    }
    fn serialize_u32(self, v: u32) -> Result<Self::Ok, Self::Error> {
        self.wrap_scalar(&v.to_string());
        Ok(())
    }
    fn serialize_u64(self, v: u64) -> Result<Self::Ok, Self::Error> {
        self.wrap_scalar(&v.to_string());
        Ok(())
    }
    fn serialize_f32(self, v: f32) -> Result<Self::Ok, Self::Error> {
        self.wrap_scalar(&v.to_string());
        Ok(())
    }
    fn serialize_f64(self, v: f64) -> Result<Self::Ok, Self::Error> {
        self.wrap_scalar(&v.to_string());
        Ok(())
    }

    fn serialize_char(self, v: char) -> Result<Self::Ok, Self::Error> {
        self.wrap_scalar_escaped(&v.to_string());
        Ok(())
    }

    fn serialize_str(self, v: &str) -> Result<Self::Ok, Self::Error> {
        self.wrap_scalar_escaped(v);
        Ok(())
    }

    fn serialize_bytes(self, _v: &[u8]) -> Result<Self::Ok, Self::Error> {
        Err(Error::Message("bytes not supported".to_string()))
    }

    fn serialize_none(self) -> Result<Self::Ok, Self::Error> {
        Ok(())
    }

    fn serialize_some<T: ?Sized + Serialize>(self, value: &T) -> Result<Self::Ok, Self::Error> {
        value.serialize(self)
    }

    fn serialize_unit(self) -> Result<Self::Ok, Self::Error> {
        self.output.push('<');
        self.output.push_str(self.tag);
        self.output.push_str("/>");
        Ok(())
    }

    fn serialize_unit_struct(self, _name: &'static str) -> Result<Self::Ok, Self::Error> {
        self.serialize_unit()
    }

    fn serialize_unit_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        variant: &'static str,
    ) -> Result<Self::Ok, Self::Error> {
        self.wrap_scalar(variant);
        Ok(())
    }

    fn serialize_newtype_struct<T: ?Sized + Serialize>(
        self,
        _name: &'static str,
        value: &T,
    ) -> Result<Self::Ok, Self::Error> {
        value.serialize(self)
    }

    fn serialize_newtype_variant<T: ?Sized + Serialize>(
        self,
        _name: &'static str,
        _variant_index: u32,
        _variant: &'static str,
        value: &T,
    ) -> Result<Self::Ok, Self::Error> {
        value.serialize(self)
    }

    fn serialize_seq(self, _len: Option<usize>) -> Result<Self::SerializeSeq, Self::Error> {
        Ok(SeqFieldSerializer {
            output: self.output,
            tag: self.tag,
        })
    }

    fn serialize_tuple(self, _len: usize) -> Result<Self::SerializeTuple, Self::Error> {
        Ok(SeqFieldSerializer {
            output: self.output,
            tag: self.tag,
        })
    }

    fn serialize_tuple_struct(
        self,
        _name: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeTupleStruct, Self::Error> {
        Ok(SeqFieldSerializer {
            output: self.output,
            tag: self.tag,
        })
    }

    fn serialize_tuple_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        _variant: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeTupleVariant, Self::Error> {
        Ok(SeqFieldSerializer {
            output: self.output,
            tag: self.tag,
        })
    }

    fn serialize_map(self, _len: Option<usize>) -> Result<Self::SerializeMap, Self::Error> {
        Ok(MapSerializer {
            output: self.output,
            current_key: None,
        })
    }

    fn serialize_struct(
        self,
        _name: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeStruct, Self::Error> {
        Ok(StructSerializer {
            output: self.output,
            tag: self.tag.to_string(),
            attrs: String::new(),
            body: String::new(),
        })
    }

    fn serialize_struct_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        variant: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeStructVariant, Self::Error> {
        Ok(StructSerializer {
            output: self.output,
            tag: variant.to_string(),
            attrs: String::new(),
            body: String::new(),
        })
    }
}

/// Sequence serializer for top-level seq.
struct SeqSerializer<'a> {
    output: &'a mut String,
}

impl ser::SerializeSeq for SeqSerializer<'_> {
    type Ok = ();
    type Error = Error;

    fn serialize_element<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<(), Self::Error> {
        value.serialize(XmlSerializer {
            output: self.output,
        })
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        Ok(())
    }
}

impl ser::SerializeTuple for SeqSerializer<'_> {
    type Ok = ();
    type Error = Error;

    fn serialize_element<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<(), Self::Error> {
        ser::SerializeSeq::serialize_element(self, value)
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        ser::SerializeSeq::end(self)
    }
}

impl ser::SerializeTupleStruct for SeqSerializer<'_> {
    type Ok = ();
    type Error = Error;

    fn serialize_field<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<(), Self::Error> {
        ser::SerializeSeq::serialize_element(self, value)
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        ser::SerializeSeq::end(self)
    }
}

impl ser::SerializeTupleVariant for SeqSerializer<'_> {
    type Ok = ();
    type Error = Error;

    fn serialize_field<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<(), Self::Error> {
        ser::SerializeSeq::serialize_element(self, value)
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        ser::SerializeSeq::end(self)
    }
}

/// Sequence field serializer: each element gets wrapped in `<tag>...</tag>`.
struct SeqFieldSerializer<'a> {
    output: &'a mut String,
    tag: &'a str,
}

impl ser::SerializeSeq for SeqFieldSerializer<'_> {
    type Ok = ();
    type Error = Error;

    fn serialize_element<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<(), Self::Error> {
        value.serialize(FieldSerializer {
            output: self.output,
            tag: self.tag,
        })
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        Ok(())
    }
}

impl ser::SerializeTuple for SeqFieldSerializer<'_> {
    type Ok = ();
    type Error = Error;

    fn serialize_element<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<(), Self::Error> {
        ser::SerializeSeq::serialize_element(self, value)
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        ser::SerializeSeq::end(self)
    }
}

impl ser::SerializeTupleStruct for SeqFieldSerializer<'_> {
    type Ok = ();
    type Error = Error;

    fn serialize_field<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<(), Self::Error> {
        ser::SerializeSeq::serialize_element(self, value)
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        ser::SerializeSeq::end(self)
    }
}

impl ser::SerializeTupleVariant for SeqFieldSerializer<'_> {
    type Ok = ();
    type Error = Error;

    fn serialize_field<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<(), Self::Error> {
        ser::SerializeSeq::serialize_element(self, value)
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        ser::SerializeSeq::end(self)
    }
}

/// Map serializer.
struct MapSerializer<'a> {
    output: &'a mut String,
    current_key: Option<String>,
}

impl ser::SerializeMap for MapSerializer<'_> {
    type Ok = ();
    type Error = Error;

    fn serialize_key<T: ?Sized + Serialize>(&mut self, key: &T) -> Result<(), Self::Error> {
        let mut key_str = String::new();
        key.serialize(XmlSerializer {
            output: &mut key_str,
        })?;
        self.current_key = Some(key_str);
        Ok(())
    }

    fn serialize_value<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<(), Self::Error> {
        let key = self
            .current_key
            .take()
            .ok_or_else(|| Error::Message("serialize_value called without key".to_string()))?;
        self.output.push('<');
        self.output.push_str(&key);
        self.output.push('>');
        value.serialize(XmlSerializer {
            output: self.output,
        })?;
        self.output.push_str("</");
        self.output.push_str(&key);
        self.output.push('>');
        Ok(())
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        Ok(())
    }
}

/// Escape XML special characters for text content.
fn escape_xml_to(output: &mut String, s: &str) {
    for c in s.chars() {
        match c {
            '<' => output.push_str("&lt;"),
            '>' => output.push_str("&gt;"),
            '&' => output.push_str("&amp;"),
            _ => output.push(c),
        }
    }
}

/// Escape XML special characters for attribute values.
fn escape_xml_attr_to(output: &mut String, s: &str) {
    for c in s.chars() {
        match c {
            '<' => output.push_str("&lt;"),
            '>' => output.push_str("&gt;"),
            '&' => output.push_str("&amp;"),
            '"' => output.push_str("&quot;"),
            _ => output.push(c),
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use serde::Serialize;

    #[test]
    fn test_ser_simple_struct() {
        #[derive(Serialize)]
        #[serde(rename = "root")]
        struct Root {
            name: String,
            value: String,
        }
        let r = Root {
            name: "hello".to_string(),
            value: "world".to_string(),
        };
        let xml = to_string(&r).unwrap();
        assert_eq!(xml, "<root><name>hello</name><value>world</value></root>");
    }

    #[test]
    fn test_ser_attributes() {
        #[derive(Serialize)]
        #[serde(rename = "item")]
        struct Item {
            #[serde(rename = "$attr:id")]
            id: String,
            #[serde(rename = "$attr:class")]
            class: String,
        }
        let item = Item {
            id: "1".to_string(),
            class: "foo".to_string(),
        };
        let xml = to_string(&item).unwrap();
        assert_eq!(xml, r#"<item id="1" class="foo"/>"#);
    }

    #[test]
    fn test_ser_text_content() {
        #[derive(Serialize)]
        #[serde(rename = "msg")]
        struct Msg {
            #[serde(rename = "$text")]
            text: String,
        }
        let msg = Msg {
            text: "Hello World".to_string(),
        };
        let xml = to_string(&msg).unwrap();
        assert_eq!(xml, "<msg>Hello World</msg>");
    }

    #[test]
    fn test_ser_sequence() {
        #[derive(Serialize)]
        #[serde(rename = "item")]
        struct Item {
            #[serde(rename = "$text")]
            text: String,
        }
        #[derive(Serialize)]
        #[serde(rename = "list")]
        struct List {
            item: Vec<Item>,
        }
        let list = List {
            item: vec![
                Item {
                    text: "A".to_string(),
                },
                Item {
                    text: "B".to_string(),
                },
            ],
        };
        let xml = to_string(&list).unwrap();
        assert_eq!(xml, "<list><item>A</item><item>B</item></list>");
    }

    #[test]
    fn test_ser_numeric() {
        #[derive(Serialize)]
        #[serde(rename = "data")]
        struct Data {
            count: u32,
            ratio: f64,
        }
        let d = Data {
            count: 42,
            ratio: 2.72,
        };
        let xml = to_string(&d).unwrap();
        assert_eq!(xml, "<data><count>42</count><ratio>2.72</ratio></data>");
    }

    #[test]
    fn test_ser_escaping() {
        #[derive(Serialize)]
        #[serde(rename = "msg")]
        struct Msg {
            #[serde(rename = "$text")]
            text: String,
        }
        let msg = Msg {
            text: "<b>&amp;</b>".to_string(),
        };
        let xml = to_string(&msg).unwrap();
        assert_eq!(xml, "<msg>&lt;b&gt;&amp;amp;&lt;/b&gt;</msg>");
    }

    #[test]
    fn test_ser_attr_escaping() {
        #[derive(Serialize)]
        #[serde(rename = "item")]
        struct Item {
            #[serde(rename = "$attr:val")]
            val: String,
        }
        let item = Item {
            val: "a\"b".to_string(),
        };
        let xml = to_string(&item).unwrap();
        assert_eq!(xml, r#"<item val="a&quot;b"/>"#);
    }

    #[test]
    fn test_ser_nested() {
        #[derive(Serialize)]
        #[serde(rename = "inner")]
        struct Inner {
            #[serde(rename = "$text")]
            text: String,
        }
        #[derive(Serialize)]
        #[serde(rename = "outer")]
        struct Outer {
            inner: Inner,
        }
        let o = Outer {
            inner: Inner {
                text: "data".to_string(),
            },
        };
        let xml = to_string(&o).unwrap();
        assert_eq!(xml, "<outer><inner>data</inner></outer>");
    }

    #[test]
    fn test_ser_none_omitted() {
        #[derive(Serialize)]
        #[serde(rename = "data")]
        struct Data {
            #[serde(skip_serializing_if = "Option::is_none")]
            value: Option<String>,
            name: String,
        }
        let d = Data {
            value: None,
            name: "test".to_string(),
        };
        let xml = to_string(&d).unwrap();
        assert_eq!(xml, "<data><name>test</name></data>");
    }
}
