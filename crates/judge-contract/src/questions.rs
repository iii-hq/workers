use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeMap;

/// Structured provider descriptions; top-level numbers/bools are not descriptions.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum Content {
    Text(String),
    Object(Map<String, Value>),
    Array(Vec<Value>),
    #[default]
    Null,
}
impl From<String> for Content {
    fn from(value: String) -> Self {
        Self::Text(value)
    }
}
impl From<&str> for Content {
    fn from(value: &str) -> Self {
        Self::Text(value.into())
    }
}

/// Score levels require a description. Unlike instructions, Choice option
/// descriptions and Noul criteria, the provider rejects a null level (HTTP422).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum ScoreLevel {
    Text(String),
    Object(Map<String, Value>),
    Array(Vec<Value>),
}
impl From<String> for ScoreLevel {
    fn from(value: String) -> Self {
        Self::Text(value)
    }
}
impl From<&str> for ScoreLevel {
    fn from(value: &str) -> Self {
        Self::Text(value.into())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Question {
    Noul {
        #[serde(default)]
        instructions: Content,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[schemars(schema_with = "noul_criteria_schema")]
        criteria: Option<BTreeMap<String, Content>>,
    },
    Choice {
        #[serde(default)]
        instructions: Content,
        #[schemars(schema_with = "choice_criteria_schema")]
        criteria: BTreeMap<String, Content>,
    },
    Score {
        #[serde(default)]
        instructions: Content,
        #[schemars(length(min = 2, max = 10))]
        criteria: Vec<ScoreLevel>,
    },
}

pub(crate) fn questions_schema(
    generator: &mut schemars::gen::SchemaGenerator,
) -> schemars::schema::Schema {
    map_schema::<Question>(generator, 1, None)
}
fn choice_criteria_schema(
    generator: &mut schemars::gen::SchemaGenerator,
) -> schemars::schema::Schema {
    map_schema::<Content>(generator, 1, Some(255))
}
fn map_schema<T: JsonSchema>(
    generator: &mut schemars::gen::SchemaGenerator,
    min: u32,
    max: Option<u32>,
) -> schemars::schema::Schema {
    let mut schema = BTreeMap::<String, T>::json_schema(generator).into_object();
    let validation = schema.object.get_or_insert_with(Default::default);
    validation.min_properties = Some(min);
    validation.max_properties = max;
    schema.into()
}
fn noul_criteria_schema(
    generator: &mut schemars::gen::SchemaGenerator,
) -> schemars::schema::Schema {
    use schemars::schema::{
        InstanceType, ObjectValidation, Schema, SchemaObject, SubschemaValidation,
    };
    let object = SchemaObject {
        instance_type: Some(InstanceType::Object.into()),
        object: Some(Box::new(ObjectValidation {
            properties: ["true", "false"]
                .into_iter()
                .map(|key| (key.into(), generator.subschema_for::<Content>()))
                .collect(),
            additional_properties: Some(Box::new(Schema::Bool(false))),
            ..Default::default()
        })),
        ..Default::default()
    };
    SchemaObject {
        subschemas: Some(Box::new(SubschemaValidation {
            any_of: Some(vec![
                object.into(),
                SchemaObject {
                    instance_type: Some(InstanceType::Null.into()),
                    ..Default::default()
                }
                .into(),
            ]),
            ..Default::default()
        })),
        ..Default::default()
    }
    .into()
}
