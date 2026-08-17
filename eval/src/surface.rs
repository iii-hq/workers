use schemars::JsonSchema;

use crate::comparison::{CompareSessionsRequestV1, SessionComparisonResponseV1};
use crate::contract::{
    EvalCancelResponseV1, EvalDeleteResponseV1, EvalListRequestV1, EvalListResponseV1,
    EvalRerunRequestV1, EvalResultResponseV1, EvalStartRequestV1, EvalStartResponseV1,
    EvalStatusResponseV1, EvaluationIdRequestV1, EvaluatorInputV1, EvaluatorResponseV1,
    StepRequestV1, StepResponseV1, SweepEventV1, SweepResponseV1, WakeEventV1, WakeResponseV1,
};
use crate::functions::{
    CANCEL_ID, COMPARE_SESSIONS_ID, DELETE_ID, EXACT_ID, LIST_ID, NORMALIZED_TEXT_ID, RERUN_ID,
    RESULT_ID, START_ID, STATUS_ID, STEP_ID, SWEEP_ID, WAKE_ID,
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
        spec::<CompareSessionsRequestV1, SessionComparisonResponseV1>(COMPARE_SESSIONS_ID),
        spec::<EvalStartRequestV1, EvalStartResponseV1>(START_ID),
        spec::<EvalRerunRequestV1, EvalStartResponseV1>(RERUN_ID),
        spec::<EvalListRequestV1, EvalListResponseV1>(LIST_ID),
        spec::<EvaluationIdRequestV1, Option<EvalStatusResponseV1>>(STATUS_ID),
        spec::<EvaluationIdRequestV1, Option<EvalResultResponseV1>>(RESULT_ID),
        spec::<EvaluationIdRequestV1, EvalCancelResponseV1>(CANCEL_ID),
        spec::<EvaluationIdRequestV1, EvalDeleteResponseV1>(DELETE_ID),
        spec::<EvaluatorInputV1, EvaluatorResponseV1>(EXACT_ID),
        spec::<EvaluatorInputV1, EvaluatorResponseV1>(NORMALIZED_TEXT_ID),
        spec::<StepRequestV1, StepResponseV1>(STEP_ID),
        spec::<WakeEventV1, WakeResponseV1>(WAKE_ID),
        spec::<SweepEventV1, SweepResponseV1>(SWEEP_ID),
    ]
}
