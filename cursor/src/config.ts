import { resolve } from 'node:path';
import { z } from 'zod';
import type { BridgeLaunchOptions } from './bridge.js';
import type { CursorCliLaunchOptions } from './cli.js';

export const API_KEY_ENV_REFERENCE = '$' + '{CURSOR_API_KEY:}';
export const BRIDGE_BIN_ENV_REFERENCE = '$' + '{CURSOR_SDK_BRIDGE_BIN:}';
export const CURSOR_AGENT_BIN_ENV_REFERENCE = '$' + '{CURSOR_AGENT_BIN:}';

export const ConfigSchema = z.object({
  local_backend: z
    .enum(['cli-acp', 'sdk-bridge'])
    .default('cli-acp')
    .describe('Local Cursor backend; cli-acp reuses the normal Cursor Agent login'),
  agent_binary: z
    .string()
    .default(CURSOR_AGENT_BIN_ENV_REFERENCE)
    .describe('Optional path to the official Cursor Agent CLI binary'),
  api_key: z
    .string()
    .default(API_KEY_ENV_REFERENCE)
    .describe('Cursor API key environment reference used only by the SDK Bridge backend'),
  bridge_binary: z
    .string()
    .default(BRIDGE_BIN_ENV_REFERENCE)
    .describe('Path to a separately installed cursor-sdk-bridge v1.0.28 binary'),
  workspace: z.string().default('.').describe('Default workspace used for cloud bridge processes'),
  startup_timeout_ms: z.number().int().positive().default(30_000),
  shutdown_timeout_ms: z.number().int().positive().default(5_000),
  rpc_timeout_ms: z.number().int().positive().default(60_000),
  max_frame_bytes: z
    .number()
    .int()
    .positive()
    .default(16 * 1024 * 1024),
  events_stream: z.string().min(1).default('agent::events'),
  raw_events_stream: z.string().min(1).default('cursor::events'),
});

export type Config = z.infer<typeof ConfigSchema>;
export type ConfigHolder = { current: Config };

export function defaultConfig(): Config {
  return ConfigSchema.parse({});
}

export function configId(): string {
  return process.env.III_CONFIG_NAME?.trim() || 'cursor';
}

export function runtimeJsonSchema(): Record<string, unknown> {
  const schema = z.toJSONSchema(ConfigSchema) as Record<string, unknown>;
  delete schema.$schema;
  schema.example = defaultConfig();
  return schema;
}

export function requireApiKey(config: Config): string {
  const key = config.api_key.trim();
  if (!key || key === API_KEY_ENV_REFERENCE) {
    throw new Error(
      'Cursor API key is not configured; set CURSOR_API_KEY or update the cursor configuration entry',
    );
  }
  return key;
}

export function bridgeLaunchOptions(config: Config, workspace?: string): BridgeLaunchOptions {
  return {
    binary: config.bridge_binary,
    workspace: resolve(workspace || config.workspace),
    apiKey: requireApiKey(config),
    startupTimeoutMs: config.startup_timeout_ms,
    shutdownTimeoutMs: config.shutdown_timeout_ms,
    rpcTimeoutMs: config.rpc_timeout_ms,
    maxFrameBytes: config.max_frame_bytes,
  };
}

export function cursorCliLaunchOptions(config: Config, workspace?: string): CursorCliLaunchOptions {
  const configured = config.agent_binary.trim();
  const fromEnvironment = process.env.CURSOR_AGENT_BIN?.trim();
  return {
    binary:
      configured && configured !== CURSOR_AGENT_BIN_ENV_REFERENCE
        ? configured
        : fromEnvironment || '',
    workspace: resolve(workspace || config.workspace),
    startupTimeoutMs: config.startup_timeout_ms,
    shutdownTimeoutMs: config.shutdown_timeout_ms,
    rpcTimeoutMs: config.rpc_timeout_ms,
    maxFrameBytes: config.max_frame_bytes,
  };
}
