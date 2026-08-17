import { z } from 'zod';

function jsonSchema(schema) {
  const out = z.toJSONSchema(schema);
  delete out.$schema;
  return out;
}

const MiddlewarePayloadSchema = z
  .object({
    verdict_token: z.string().optional(),
    branch: z.string().optional(),
  })
  .passthrough();

const MiddlewareContextSchema = z
  .object({
    msn_id: z.string().optional(),
    holder_id: z.string().optional(),
    repo_root: z.string().optional(),
    worktree_path: z.string().optional(),
    mission_rel_path: z.string().optional(),
    mission_rel: z.string().optional(),
    verdict_token: z.string().optional(),
  })
  .passthrough();

export const MiddlewareRequestSchema = z
  .object({
    function_id: z.string(),
    payload: MiddlewarePayloadSchema.optional(),
    context: MiddlewareContextSchema.optional(),
  })
  .strict();

export const MiddlewareResponseSchema = z.union([
  z.object({}).passthrough(),
  z.array(z.unknown()),
  z.string(),
  z.number(),
  z.boolean(),
  z.null(),
]);

export const VerifyRequestSchema = z
  .object({
    repo_root: z.string(),
    msn_id: z.string().optional(),
    mission_rel_path: z.string().optional(),
    options: z.record(z.string(), z.unknown()).optional(),
  })
  .strict();

const VerifyFindingSchema = z
  .object({
    failed_gate: z.string().optional(),
    offending_file: z.string().optional(),
    line: z.number().optional(),
    severity: z.string().optional(),
    resolution_hint: z.string().optional(),
  })
  .strict();

export const VerifyResponseSchema = z
  .object({
    status: z.string(),
    phase: z.string().optional(),
    message: z.string().optional(),
    error_code: z.string().optional(),
    exit_code: z.number().optional(),
    msn_id: z.string().optional(),
    mission_file_path: z.string().optional(),
    findings: z.array(VerifyFindingSchema).optional(),
    fix_hints: z.array(z.string()).optional(),
    next_actions: z.array(z.string()).optional(),
  })
  .strict();

export const OnFunctionRegistrationRequestSchema = z
  .object({
    function_id: z.string(),
  })
  .strict();

export const OnFunctionRegistrationResponseSchema = z
  .object({
    function_id: z.string(),
  })
  .strict();

export const OnTriggerRegistrationRequestSchema = z
  .object({
    function_id: z.string(),
  })
  .strict();

export const OnTriggerRegistrationResponseSchema = z
  .object({
    function_id: z.string(),
  })
  .strict();

export const OnTriggerTypeRegistrationRequestSchema = z
  .object({
    type_id: z.string(),
  })
  .strict();

export const OnTriggerTypeRegistrationResponseSchema = z
  .object({
    denied: z.literal(true),
  })
  .strict();

export const MIDDLEWARE_REQUEST_FORMAT = jsonSchema(MiddlewareRequestSchema);
export const MIDDLEWARE_RESPONSE_FORMAT = jsonSchema(MiddlewareResponseSchema);
export const VERIFY_REQUEST_FORMAT = jsonSchema(VerifyRequestSchema);
export const VERIFY_RESPONSE_FORMAT = jsonSchema(VerifyResponseSchema);
export const ON_FUNCTION_REGISTRATION_REQUEST_FORMAT = jsonSchema(
  OnFunctionRegistrationRequestSchema,
);
export const ON_FUNCTION_REGISTRATION_RESPONSE_FORMAT = jsonSchema(
  OnFunctionRegistrationResponseSchema,
);
export const ON_TRIGGER_REGISTRATION_REQUEST_FORMAT = jsonSchema(
  OnTriggerRegistrationRequestSchema,
);
export const ON_TRIGGER_REGISTRATION_RESPONSE_FORMAT = jsonSchema(
  OnTriggerRegistrationResponseSchema,
);
export const ON_TRIGGER_TYPE_REGISTRATION_REQUEST_FORMAT = jsonSchema(
  OnTriggerTypeRegistrationRequestSchema,
);
export const ON_TRIGGER_TYPE_REGISTRATION_RESPONSE_FORMAT = jsonSchema(
  OnTriggerTypeRegistrationResponseSchema,
);
