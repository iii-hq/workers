use schemars::JsonSchema;

use crate::contract::{
    AnalysisRecordV1, AnalyzeSessionRequestV1, AnalyzeSessionResponseV1, AttachValidationRequestV1,
    AttachValidationResponseV1, ConfigureRequestV1, EvalCancelResponseV1, EvalDeleteResponseV1,
    EvalListRequestV1, EvalListResponseV1, EvalResultResponseV1, EvaluationIdRequestV1,
    MonitorConfigV1, MonitorStateRequestV1, MonitorStateResponseV1, ProposeValidationRequestV1,
    ProposeValidationResponseV1, RecurrenceRequestV1, RecurrenceResponseV1, ReproduceRequestV1,
    ReproduceResponseV1, ReviewRequestV1, ReviewsRequestV1, ReviewsResponseV1,
    StartValidationRequestV1, StartValidationResponseV1, StepRequestV1, StepResponseV1,
    SuggestionReviewV1, SweepEventV1, SweepResponseV1, WakeEventV1, WakeResponseV1,
};
use crate::functions::{
    ANALYZE_SESSION_ID, ATTACH_VALIDATION_ID, CANCEL_ID, CONFIGURE_ID, CONFIG_ID, DELETE_ID,
    LIST_ID, PROPOSE_VALIDATION_ID, RECURRENCE_ID, REPRODUCE_ID, RESULT_ID, REVIEWS_ID, REVIEW_ID,
    START_VALIDATION_ID, STATUS_ID, STEP_ID, SWEEP_ID, WAKE_ID,
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
        spec::<ConfigureRequestV1, MonitorConfigV1>(CONFIGURE_ID),
        spec::<MonitorStateRequestV1, MonitorStateResponseV1>(CONFIG_ID),
        spec::<AnalyzeSessionRequestV1, AnalyzeSessionResponseV1>(ANALYZE_SESSION_ID),
        spec::<EvalListRequestV1, EvalListResponseV1>(LIST_ID),
        spec::<EvaluationIdRequestV1, Option<AnalysisRecordV1>>(STATUS_ID),
        spec::<EvaluationIdRequestV1, Option<EvalResultResponseV1>>(RESULT_ID),
        spec::<EvaluationIdRequestV1, EvalCancelResponseV1>(CANCEL_ID),
        spec::<EvaluationIdRequestV1, EvalDeleteResponseV1>(DELETE_ID),
        spec::<AttachValidationRequestV1, AttachValidationResponseV1>(ATTACH_VALIDATION_ID),
        spec::<ProposeValidationRequestV1, ProposeValidationResponseV1>(PROPOSE_VALIDATION_ID),
        spec::<StartValidationRequestV1, StartValidationResponseV1>(START_VALIDATION_ID),
        spec::<ReviewRequestV1, SuggestionReviewV1>(REVIEW_ID),
        spec::<ReviewsRequestV1, ReviewsResponseV1>(REVIEWS_ID),
        spec::<RecurrenceRequestV1, RecurrenceResponseV1>(RECURRENCE_ID),
        spec::<ReproduceRequestV1, ReproduceResponseV1>(REPRODUCE_ID),
        spec::<StepRequestV1, StepResponseV1>(STEP_ID),
        spec::<WakeEventV1, WakeResponseV1>(WAKE_ID),
        spec::<SweepEventV1, SweepResponseV1>(SWEEP_ID),
    ]
}
