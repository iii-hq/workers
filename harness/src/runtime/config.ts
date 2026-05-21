/**
 * YAML config loader. Each worker calls `loadConfig(path)` to read
 * `config.yaml` and pluck whichever subsection it cares about. Missing
 * file / parse failure → empty object so callers fall back to their
 * own defaults.
 */

import { readFile } from 'node:fs/promises';
import { parse as parseYaml } from 'yaml';
import { logger } from './otel.js';

export async function loadConfig(path: string): Promise<Record<string, unknown>> {
  try {
    const text = await readFile(path, 'utf8');
    const parsed = parseYaml(text);
    return parsed && typeof parsed === 'object' ? (parsed as Record<string, unknown>) : {};
  } catch (err) {
    logger.warn('config load failed; using defaults', { path, err: String(err) });
    return {};
  }
}

export function getString(cfg: Record<string, unknown>, key: string, fallback: string): string {
  const v = cfg[key];
  return typeof v === 'string' && v.length > 0 ? v : fallback;
}

export function getNumber(cfg: Record<string, unknown>, key: string, fallback: number): number {
  const v = cfg[key];
  return typeof v === 'number' && Number.isFinite(v) ? v : fallback;
}

export function getStringArray(
  cfg: Record<string, unknown>,
  key: string,
  fallback: string[],
): string[] {
  const v = cfg[key];
  return Array.isArray(v) && v.every((s) => typeof s === 'string') ? (v as string[]) : fallback;
}

export function getSection(cfg: Record<string, unknown>, key: string): Record<string, unknown> {
  const v = cfg[key];
  return typeof v === 'object' && v !== null ? (v as Record<string, unknown>) : {};
}
