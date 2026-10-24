use schemars::JsonSchema;

use crate::contract::{
    ConfigRequestV1, ConfigResponseV1, ConfigureRequestV1, CycleCancelResponseV1, CycleIdRequestV1,
    CycleListRequestV1, CycleListResponseV1, CycleRecordV1, CycleResultResponseV1,
    CycleStartRequestV1, CycleStartResponseV1, ExecutionAdmissionResponseV1,
    ExecutionChangedEventV1, StepRequestV1, StepResponseV1, StoredConfigV1, SweepEventV1,
    SweepResponseV1,
};
use crate::functions::{
    CANCEL_ID, CONFIGURE_ID, CONFIG_ID, EXECUTION_CHANGED_ID, LIST_ID, RESULT_ID, START_ID,
    STATUS_ID, STEP_ID, SWEEP_ID,
};

pub struct FunctionSpec {
    pub function_id: &'static str,
    pub request_schema: schemars::schema::RootSchema,
    pub response_schema: schemars::schema::RootSchema,
}

fn schema_of<T: JsonSchema>() -> schemars::schema::RootSchema {
    schemars::r#gen::SchemaSettings::draft07()
        .into_generator()
        .into_root_schema_for::<T>()
}

fn spec<Req: JsonSchema, Resp: JsonSchema>(function_id: &'static str) -> FunctionSpec {
    FunctionSpec {
        function_id,
        request_schema: schema_of::<Req>(),
        response_schema: schema_of::<Resp>(),
    }
}

pub fn catalog() -> Vec<FunctionSpec> {
    vec![
        spec::<ConfigureRequestV1, StoredConfigV1>(CONFIGURE_ID),
        spec::<ConfigRequestV1, ConfigResponseV1>(CONFIG_ID),
        spec::<CycleStartRequestV1, CycleStartResponseV1>(START_ID),
        spec::<CycleListRequestV1, CycleListResponseV1>(LIST_ID),
        spec::<CycleIdRequestV1, Option<CycleRecordV1>>(STATUS_ID),
        spec::<CycleIdRequestV1, Option<CycleResultResponseV1>>(RESULT_ID),
        spec::<CycleIdRequestV1, CycleCancelResponseV1>(CANCEL_ID),
        spec::<StepRequestV1, StepResponseV1>(STEP_ID),
        spec::<SweepEventV1, SweepResponseV1>(SWEEP_ID),
        spec::<ExecutionChangedEventV1, ExecutionAdmissionResponseV1>(EXECUTION_CHANGED_ID),
    ]
}
