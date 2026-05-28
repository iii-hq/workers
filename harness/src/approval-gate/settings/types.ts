import { z } from 'zod';
import type { ApprovalSettings } from '../schemas.js';

export type { ApprovalSettings, PermissionMode } from '../schemas.js';

export type GetSettingsReply = ApprovalSettings;
export type MutationReply = { ok: true } | { ok: false; error: string };


export const sessionIdField = z
  .string()
  .min(1)
  .refine((v) => !v.includes('/'), 'session_id must not contain "/"');

export const functionIdField = z.string().min(1);
