/**
 * Credential store backends.
 *
 * Two implementations:
 *
 * 1. `InMemoryStore` — used in tests and for local-only sessions.
 * 2. `FileStore` — JSON file at `~/.iii/auth-credentials.json` (path
 *    configurable). All mutations serialize through a single in-process
 *    promise queue so concurrent set/clear calls don't clobber each other.
 *    Atomic write via `tmp` + `rename`.
 *
 * The Rust crate uses iii-state CAS for multi-process safety. The Node
 * port is single-process by default — operators can run a single
 * `auth-credentials` worker that all other workers query via
 * `auth::get_token`.
 */

import { mkdir, readFile, rename, writeFile } from 'node:fs/promises';
import { homedir } from 'node:os';
import { dirname, resolve as resolvePath } from 'node:path';
import type { Credential } from './types.js';

export interface CredentialStore {
  get(provider: string): Promise<Credential | null>;
  set(provider: string, credential: Credential): Promise<void>;
  clear(provider: string): Promise<void>;
  list(): Promise<Array<readonly [string, Credential]>>;
}

export class InMemoryStore implements CredentialStore {
  private readonly store = new Map<string, Credential>();
  async get(provider: string): Promise<Credential | null> {
    return this.store.get(provider) ?? null;
  }
  async set(provider: string, credential: Credential): Promise<void> {
    this.store.set(provider, credential);
  }
  async clear(provider: string): Promise<void> {
    this.store.delete(provider);
  }
  async list(): Promise<Array<readonly [string, Credential]>> {
    return [...this.store.entries()];
  }
}

/** Resolve `~` in a path. */
export function expandHome(p: string): string {
  if (p.startsWith('~/')) return resolvePath(homedir(), p.slice(2));
  if (p === '~') return homedir();
  return resolvePath(p);
}

export class FileStore implements CredentialStore {
  private readonly path: string;
  private writeChain: Promise<void> = Promise.resolve();

  constructor(path: string) {
    this.path = expandHome(path);
  }

  async get(provider: string): Promise<Credential | null> {
    const map = await this.read();
    return map[provider] ?? null;
  }

  async list(): Promise<Array<readonly [string, Credential]>> {
    const map = await this.read();
    return Object.entries(map).map(([k, v]) => [k, v] as const);
  }

  async set(provider: string, credential: Credential): Promise<void> {
    await this.queue(async () => {
      const map = await this.read();
      map[provider] = credential;
      await this.write(map);
    });
  }

  async clear(provider: string): Promise<void> {
    await this.queue(async () => {
      const map = await this.read();
      if (provider in map) {
        delete map[provider];
        await this.write(map);
      }
    });
  }

  private queue<T>(op: () => Promise<T>): Promise<T> {
    const next = this.writeChain.then(() => op());
    this.writeChain = next.then(
      () => undefined,
      () => undefined,
    );
    return next;
  }

  private async read(): Promise<Record<string, Credential>> {
    try {
      const text = await readFile(this.path, 'utf8');
      const parsed = JSON.parse(text);
      if (parsed && typeof parsed === 'object') {
        return parsed as Record<string, Credential>;
      }
      return {};
    } catch (err) {
      if ((err as NodeJS.ErrnoException).code === 'ENOENT') return {};
      throw err;
    }
  }

  private async write(map: Record<string, Credential>): Promise<void> {
    await mkdir(dirname(this.path), { recursive: true, mode: 0o700 });
    const tmp = `${this.path}.tmp-${process.pid}-${Date.now()}`;
    await writeFile(tmp, `${JSON.stringify(map, null, 2)}\n`, { mode: 0o600 });
    await rename(tmp, this.path);
  }
}
