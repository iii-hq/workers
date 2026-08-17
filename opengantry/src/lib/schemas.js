import gantryMiddleware from '../../schemas/gantry__middleware.json' with { type: 'json' };
import gantryMiddlewareResponse from '../../schemas/gantry__middleware.response.json' with {
  type: 'json',
};
import gantryVerify from '../../schemas/gantry__verify.json' with { type: 'json' };
import gantryVerifyResponse from '../../schemas/gantry__verify.response.json' with { type: 'json' };
import gantryOnFunctionRegistration from '../../schemas/gantry__on-function-registration.json' with {
  type: 'json',
};
import gantryOnFunctionRegistrationResponse from '../../schemas/gantry__on-function-registration.response.json' with {
  type: 'json',
};
import gantryOnTriggerRegistration from '../../schemas/gantry__on-trigger-registration.json' with {
  type: 'json',
};
import gantryOnTriggerRegistrationResponse from '../../schemas/gantry__on-trigger-registration.response.json' with {
  type: 'json',
};
import gantryOnTriggerTypeRegistration from '../../schemas/gantry__on-trigger-type-registration.json' with {
  type: 'json',
};
import gantryOnTriggerTypeRegistrationResponse from '../../schemas/gantry__on-trigger-type-registration.response.json' with {
  type: 'json',
};

const SCHEMAS = {
  'gantry__middleware.json': gantryMiddleware,
  'gantry__middleware.response.json': gantryMiddlewareResponse,
  'gantry__verify.json': gantryVerify,
  'gantry__verify.response.json': gantryVerifyResponse,
  'gantry__on-function-registration.json': gantryOnFunctionRegistration,
  'gantry__on-function-registration.response.json': gantryOnFunctionRegistrationResponse,
  'gantry__on-trigger-registration.json': gantryOnTriggerRegistration,
  'gantry__on-trigger-registration.response.json': gantryOnTriggerRegistrationResponse,
  'gantry__on-trigger-type-registration.json': gantryOnTriggerTypeRegistration,
  'gantry__on-trigger-type-registration.response.json': gantryOnTriggerTypeRegistrationResponse,
};

export function loadSchema(name) {
  const schema = SCHEMAS[name];
  if (!schema) throw new Error(`unknown schema: ${name}`);
  return schema;
}
