mod support;

use iii_sdk::errors::Error;
use schemars::{schema::RootSchema, JsonSchema};
use sentinel::events::{
    observable_catalog, observable_handler, ON_GROUP_CHANGED, ON_INVESTIGATION_CHANGED,
};
use sentinel::functions::catalog;
use sentinel::{GroupChangedEventV1, InvestigationChangedEventV1};

fn golden_file_name(function_id: &str) -> String {
    format!("schemas/{}.json", function_id.replace("::", "."))
}

#[test]
fn catalog_matches_the_registered_surface() {
    let ids: Vec<_> = catalog().iter().map(|spec| spec.function_id).collect();
    assert_eq!(
        ids,
        [
            "sentinel::status",
            "sentinel::groups::list",
            "sentinel::groups::get",
            "sentinel::groups::resolve",
            "sentinel::groups::ignore",
            "sentinel::groups::unignore",
            "sentinel::groups::reopen",
            "sentinel::occurrences::list",
            "sentinel::evidence::get",
            "sentinel::groups::history",
            "sentinel::diagnoses::list",
            "sentinel::investigate",
            "sentinel::investigations::get",
            "sentinel::investigations::list",
            "sentinel::investigations::cancel",
            "sentinel::diagnosis::record",
            "sentinel::trace::get",
            "sentinel::logs::list",
            "sentinel::on-trace-activity",
            "sentinel::on-log",
            "sentinel::ingest",
            "sentinel::on-turn-completed",
            "sentinel::on-schedule",
            "sentinel::on-config-change",
        ]
    );
}

#[test]
fn schemas_are_typed_and_match_goldens() {
    let mut failures = Vec::new();
    for spec in catalog() {
        support::assert_typed_schema(
            &format!("{} request", spec.function_id),
            &spec.request_schema,
        );
        support::assert_typed_schema(
            &format!("{} response", spec.function_id),
            &spec.response_schema,
        );
        let value = serde_json::json!({
            "function_id": spec.function_id,
            "description": spec.description,
            "request_schema": spec.request_schema,
            "response_schema": spec.response_schema,
        });
        let mut actual = serde_json::to_string_pretty(&value).expect("schema serializes");
        actual.push('\n');
        if let Err(error) = support::check_golden(&golden_file_name(spec.function_id), &actual) {
            failures.push(error);
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn observable_functions_sit_next_to_their_trigger_types() {
    let ids: Vec<_> = observable_catalog()
        .iter()
        .map(|spec| spec.function_id)
        .collect();
    assert_eq!(
        ids,
        [
            "sentinel::on-group-changed",
            "sentinel::on-investigation-changed",
        ]
    );
    // Observable functions are not invocable: none is in the call catalog.
    for spec in catalog() {
        assert!(
            !ids.contains(&spec.function_id),
            "{} is in both catalogs",
            spec.function_id
        );
    }
}

#[test]
fn observable_schemas_match_goldens() {
    let mut failures = Vec::new();
    for spec in observable_catalog() {
        assert!(
            spec.args_schema.is_object() && spec.event_schema.is_object(),
            "{} schemas are generated",
            spec.function_id
        );
        let value = serde_json::json!({
            "function_id": spec.function_id,
            "description": spec.description,
            "args_schema": spec.args_schema,
            "request_schema": spec.event_schema,
        });
        let mut actual = serde_json::to_string_pretty(&value).expect("schema serializes");
        actual.push('\n');
        if let Err(error) = support::check_golden(&golden_file_name(spec.function_id), &actual) {
            failures.push(error);
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Request / response schemas of a handler, as iii-sdk extracts them from
/// its parameter and `Ok` types at registration.
fn handler_schemas<F, T, R>(_handler: &F) -> (RootSchema, RootSchema)
where
    F: Fn(T) -> Result<R, Error>,
    T: JsonSchema,
    R: JsonSchema,
{
    fn schema<S: JsonSchema>() -> RootSchema {
        schemars::r#gen::SchemaSettings::draft07()
            .into_generator()
            .into_root_schema_for::<S>()
    }
    (schema::<T>(), schema::<R>())
}

#[test]
fn observable_handlers_publish_typed_schemas() {
    let group = observable_handler::<GroupChangedEventV1>(ON_GROUP_CHANGED);
    let investigation = observable_handler::<InvestigationChangedEventV1>(ON_INVESTIGATION_CHANGED);
    let handlers = [handler_schemas(&group), handler_schemas(&investigation)];
    let catalog = observable_catalog();
    assert_eq!(catalog.len(), handlers.len());
    for (spec, (request, response)) in catalog.iter().zip(handlers) {
        support::assert_typed_schema(&format!("{} request", spec.function_id), &request);
        support::assert_typed_schema(&format!("{} response", spec.function_id), &response);
        let request = serde_json::to_value(&request).expect("schema serializes");
        assert_ne!(request.get("title"), Some(&serde_json::json!("AnyValue")));
        assert_eq!(
            request, spec.event_schema,
            "{}: the handler's request schema is the delivered event",
            spec.function_id
        );
    }
}
