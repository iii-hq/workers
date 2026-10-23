use schemars::JsonSchema;

use crate::contract::{
    AttachValidationRequestV1, AttachValidationResponseV1, ConfigureRequestV1,
    CycleCancelResponseV1, CycleIdRequestV1, CycleListRequestV1, CycleListResponseV1,
    CycleRecordV1, CycleResultResponseV1, CycleStartRequestV1, CycleStartResponseV1,
    EvalDeleteResponseV1, EvaluationIdRequestV1, MonitorConfigV1, MonitorStateRequestV1,
    MonitorStateResponseV1, RecurrenceRequestV1, RecurrenceResponseV1, ReproduceRequestV1,
    ReproduceResponseV1, ReviewRequestV1, ReviewsRequestV1, ReviewsResponseV1,
    StartValidationRequestV1, StartValidationResponseV1, StepRequestV1, StepResponseV1,
    SuggestionReviewV1, SweepEventV1, SweepResponseV1,
};
use crate::functions::{
    ATTACH_VALIDATION_ID, CANCEL_ID, CONFIGURE_ID, CONFIG_ID, DELETE_ID, LIST_ID, RECURRENCE_ID,
    REPRODUCE_ID, RESULT_ID, REVIEWS_ID, REVIEW_ID, START_ID, START_VALIDATION_ID, STATUS_ID,
    STEP_ID, SWEEP_ID,
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
        spec::<CycleStartRequestV1, CycleStartResponseV1>(START_ID),
        spec::<CycleListRequestV1, CycleListResponseV1>(LIST_ID),
        spec::<CycleIdRequestV1, Option<CycleRecordV1>>(STATUS_ID),
        spec::<CycleIdRequestV1, Option<CycleResultResponseV1>>(RESULT_ID),
        spec::<CycleIdRequestV1, CycleCancelResponseV1>(CANCEL_ID),
        spec::<EvaluationIdRequestV1, EvalDeleteResponseV1>(DELETE_ID),
        spec::<AttachValidationRequestV1, AttachValidationResponseV1>(ATTACH_VALIDATION_ID),
        spec::<StartValidationRequestV1, StartValidationResponseV1>(START_VALIDATION_ID),
        spec::<ReviewRequestV1, SuggestionReviewV1>(REVIEW_ID),
        spec::<ReviewsRequestV1, ReviewsResponseV1>(REVIEWS_ID),
        spec::<RecurrenceRequestV1, RecurrenceResponseV1>(RECURRENCE_ID),
        spec::<ReproduceRequestV1, ReproduceResponseV1>(REPRODUCE_ID),
        spec::<StepRequestV1, StepResponseV1>(STEP_ID),
        spec::<SweepEventV1, SweepResponseV1>(SWEEP_ID),
    ]
}
