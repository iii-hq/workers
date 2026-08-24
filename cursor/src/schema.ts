import { z } from 'zod';

export function jsonSchema(schema: z.ZodType): Record<string, unknown> {
  const output = z.toJSONSchema(schema) as Record<string, unknown>;
  delete output.$schema;
  return output;
}
