import { execFile as nodeExecFile, spawn as nodeSpawn } from 'node:child_process';
import { realpath as nodeRealpath } from 'node:fs/promises';
import { homedir } from 'node:os';
import { basename, delimiter, isAbsolute, join, resolve } from 'node:path';
import type { Readable, Writable } from 'node:stream';
import { z } from 'zod';
import type { CursorModel } from './types.js';

const CURSOR_VERSION = /^\d{4}\.\d{2}\.\d{2}-[A-Za-z0-9._-]+$/;
const PUBLIC_AUTO_MODEL_ID = 'auto';
const ACP_AUTO_MODEL_ID = 'default';
const DEFAULT_MAX_FRAME_BYTES = 16 * 1024 * 1024;
const DEFAULT_SHUTDOWN_TIMEOUT_MS = 5_000;
const LONG_TURN_TIMEOUT_MS = 24 * 60 * 60 * 1_000;

const InitializeResponseSchema = z
  .object({
    protocolVersion: z.literal(1),
    authMethods: z
      .array(z.object({ id: z.string(), name: z.string().optional() }).passthrough())
      .optional(),
  })
  .passthrough();

const ModelStateSchema = z
  .object({
    currentModelId: z.string().optional(),
    availableModels: z
      .array(z.object({ modelId: z.string(), name: z.string().optional() }).passthrough())
      .default([]),
  })
  .nullable()
  .optional();

const NewSessionResponseSchema = z
  .object({
    sessionId: z.string().min(1),
    models: ModelStateSchema,
  })
  .passthrough();

const LoadSessionResponseSchema = z
  .object({
    models: ModelStateSchema,
  })
  .passthrough();

const PromptResponseSchema = z
  .object({
    stopReason: z.enum(['end_turn', 'max_tokens', 'max_turn_requests', 'refusal', 'cancelled']),
  })
  .passthrough();

const AuthStatusWireSchema = z
  .object({
    hasAccessToken: z.boolean().optional(),
    hasRefreshToken: z.boolean().optional(),
    isAuthenticated: z.boolean().optional(),
    status: z.string().optional(),
  })
  .passthrough();

type JsonObject = Record<string, unknown>;

export type CursorCliLaunchOptions = {
  binary: string;
  workspace: string;
  startupTimeoutMs: number;
  shutdownTimeoutMs: number;
  rpcTimeoutMs: number;
  maxFrameBytes: number;
};

export type CursorCliExecutable = {
  binary: string;
  version: string;
};

export type CursorCliAuthStatus = {
  authenticated: boolean;
  status: 'authenticated' | 'partial' | 'unauthenticated';
  version: string;
  login_command: 'cursor-agent login';
};

export type CursorAcpSessionUpdate = {
  sessionId: string;
  update: JsonObject;
};

export type CursorAcpSession = {
  sessionId: string;
  models: Array<{ modelId: string; name: string }>;
  currentModelId: string | null;
};

export interface CursorCliClient {
  newSession(cwd: string): Promise<CursorAcpSession>;
  loadSession(sessionId: string, cwd: string): Promise<CursorAcpSession>;
  setModel(sessionId: string, model: string): Promise<void>;
  setMode(sessionId: string, mode: 'agent' | 'plan' | 'ask'): Promise<void>;
  prompt(
    sessionId: string,
    prompt: string,
    onUpdate: (update: CursorAcpSessionUpdate) => Promise<void>,
  ): Promise<'end_turn' | 'max_tokens' | 'max_turn_requests' | 'refusal' | 'cancelled'>;
  cancel(sessionId: string): Promise<void>;
  close(): Promise<void>;
}

export interface CursorCliFactory {
  create(options: CursorCliLaunchOptions): Promise<CursorCliClient>;
  authStatus(options: CursorCliLaunchOptions): Promise<CursorCliAuthStatus>;
  listModels(options: CursorCliLaunchOptions): Promise<CursorModel[]>;
  closeAll(): Promise<void>;
  forceCloseAll(): void;
}

export type CommandResult = { stdout: string; stderr: string };
export type CommandRunner = (
  command: string,
  args: string[],
  options: { cwd?: string; env: NodeJS.ProcessEnv; timeoutMs: number },
) => Promise<CommandResult>;

type CliProcess = {
  stdin: Writable;
  stdout: Readable;
  stderr: Readable;
  exitCode: number | null;
  once(event: 'exit', listener: (code: number | null, signal: NodeJS.Signals | null) => void): void;
  once(event: 'error', listener: (error: Error) => void): void;
  once(event: 'close', listener: () => void): void;
  kill(signal?: NodeJS.Signals): boolean;
};

export type CliSpawner = (
  command: string,
  args: string[],
  options: { cwd: string; env: NodeJS.ProcessEnv; stdio: ['pipe', 'pipe', 'pipe'] },
) => CliProcess;

export type CursorCliDependencies = {
  run: CommandRunner;
  spawn: CliSpawner;
  canonicalize: (path: string) => Promise<string>;
  cwd: string;
  home: string;
  env: NodeJS.ProcessEnv;
};

type CursorCliDiscoveryDependencies = Pick<CursorCliDependencies, 'run' | 'home' | 'env'> &
  Partial<Pick<CursorCliDependencies, 'canonicalize' | 'cwd'>>;

export async function discoverCursorAgentBinary(
  configuredBinary: string,
  dependencies: CursorCliDiscoveryDependencies,
  timeoutMs = 10_000,
): Promise<CursorCliExecutable> {
  const configured = configuredBinary.trim();
  const discoveryCwd = dependencies.cwd ?? process.cwd();
  const candidates = cursorAgentCandidates(configured, dependencies, discoveryCwd);
  for (const candidate of candidates) {
    let pinned: string;
    try {
      pinned = await (dependencies.canonicalize ?? identityCanonicalPath)(candidate);
    } catch {
      continue;
    }
    if (!isAbsolute(pinned)) continue;
    const executable = await validateCursorAgent(
      pinned,
      { ...dependencies, cwd: discoveryCwd },
      timeoutMs,
    );
    if (executable) return executable;
    if (configured) {
      throw new CursorCliError(
        'Configured Cursor Agent binary is unavailable or is not the official Cursor CLI',
      );
    }
  }
  throw new CursorCliError(
    'Cursor Agent CLI was not found; install it, run cursor-agent login, or set CURSOR_AGENT_BIN',
  );
}

export function parseCursorAuthStatus(raw: string, version: string): CursorCliAuthStatus {
  let parsedJson: unknown;
  try {
    parsedJson = JSON.parse(raw);
  } catch {
    throw new CursorCliError('Cursor Agent returned an invalid authentication status');
  }
  const parsed = AuthStatusWireSchema.parse(parsedJson);
  const normalizedStatus = parsed.status?.trim().toLowerCase();
  const authenticated = parsed.isAuthenticated === true || normalizedStatus === 'authenticated';
  const partial =
    !authenticated && (parsed.hasAccessToken === true || parsed.hasRefreshToken === true);
  return {
    authenticated,
    status: authenticated ? 'authenticated' : partial ? 'partial' : 'unauthenticated',
    version,
    login_command: 'cursor-agent login',
  };
}

export function resolveCursorAcpModelId(
  requestedModel: string,
  availableModels: CursorAcpSession['models'],
): string | null {
  const modelId = requestedModel === PUBLIC_AUTO_MODEL_ID ? ACP_AUTO_MODEL_ID : requestedModel;
  return availableModels.some((model) => model.modelId === modelId) ? modelId : null;
}

export class ProductionCursorCliFactory implements CursorCliFactory {
  private readonly clients = new Set<AcpJsonRpcClient>();
  private readonly dependencies: CursorCliDependencies;

  constructor(dependencies: Partial<CursorCliDependencies> = {}) {
    this.dependencies = {
      run: dependencies.run ?? runCommand,
      spawn: dependencies.spawn ?? (nodeSpawn as unknown as CliSpawner),
      canonicalize: dependencies.canonicalize ?? nodeRealpath,
      cwd: dependencies.cwd ?? process.cwd(),
      home: dependencies.home ?? homedir(),
      env: dependencies.env ?? process.env,
    };
  }

  async create(options: CursorCliLaunchOptions): Promise<CursorCliClient> {
    const executable = await this.executable(options);
    const child = this.dependencies.spawn(executable.binary, ['acp'], {
      cwd: options.workspace,
      env: loginEnvironment(this.dependencies.env),
      stdio: ['pipe', 'pipe', 'pipe'],
    });
    const client = new AcpJsonRpcClient(
      child,
      options.startupTimeoutMs,
      options.rpcTimeoutMs,
      options.shutdownTimeoutMs,
      options.maxFrameBytes,
      () => this.clients.delete(client),
    );
    this.clients.add(client);
    try {
      await client.initialize();
      return client;
    } catch (error) {
      await client.close().catch(() => undefined);
      throw error;
    }
  }

  async authStatus(options: CursorCliLaunchOptions): Promise<CursorCliAuthStatus> {
    const executable = await this.executable(options);
    const result = await this.dependencies.run(executable.binary, ['status', '--format', 'json'], {
      cwd: options.workspace,
      env: loginEnvironment(this.dependencies.env),
      timeoutMs: options.startupTimeoutMs,
    });
    return parseCursorAuthStatus(result.stdout, executable.version);
  }

  async listModels(options: CursorCliLaunchOptions): Promise<CursorModel[]> {
    const client = await this.create(options);
    try {
      const session = await client.newSession(options.workspace);
      const models = publicCursorAcpModels(session.models);
      if (models.length === 0) {
        throw new CursorCliError(
          'Cursor Agent ACP returned no models; run cursor-agent login and verify account access',
        );
      }
      return models;
    } finally {
      await client.close().catch(() => undefined);
    }
  }

  async closeAll(): Promise<void> {
    await Promise.all([...this.clients].map((client) => client.close().catch(() => undefined)));
  }

  forceCloseAll(): void {
    for (const client of this.clients) client.forceClose();
    this.clients.clear();
  }

  private executable(options: CursorCliLaunchOptions): Promise<CursorCliExecutable> {
    return discoverCursorAgentBinary(options.binary, this.dependencies, options.startupTimeoutMs);
  }
}

export class AcpJsonRpcClient implements CursorCliClient {
  private readonly pending = new Map<
    number | string,
    {
      resolve: (value: unknown) => void;
      reject: (error: Error) => void;
      timer: ReturnType<typeof setTimeout>;
    }
  >();
  private nextId = 1;
  private closed = false;
  private updateHandler: ((update: CursorAcpSessionUpdate) => Promise<void>) | null = null;
  private activePromptSessionId: string | null = null;
  private updateQueue = Promise.resolve();
  private updateError: Error | null = null;
  private readonly processClosed: Promise<void>;
  private resolveProcessClosed: () => void = () => undefined;
  private processFinished = false;
  private closeReported = false;

  constructor(
    private readonly child: CliProcess,
    private readonly startupTimeoutMs: number,
    private readonly rpcTimeoutMs: number,
    private readonly shutdownTimeoutMs = DEFAULT_SHUTDOWN_TIMEOUT_MS,
    private readonly maxFrameBytes = DEFAULT_MAX_FRAME_BYTES,
    private readonly onClose: () => void = () => undefined,
  ) {
    this.processClosed = new Promise<void>((resolvePromise) => {
      this.resolveProcessClosed = resolvePromise;
    });
    child.stderr.resume();
    child.stdin.on('error', () => {
      this.transportFailure(new CursorCliError('Cursor Agent ACP input stream failed'));
    });
    let buffered = Buffer.alloc(0);
    child.stdout.on('data', (chunk: Buffer | string) => {
      if (this.closed) return;
      buffered = Buffer.concat([buffered, Buffer.isBuffer(chunk) ? chunk : Buffer.from(chunk)]);
      while (true) {
        const newline = buffered.indexOf(0x0a);
        if (newline < 0) break;
        const line = buffered.subarray(0, newline);
        buffered = buffered.subarray(newline + 1);
        if (line.length > this.maxFrameBytes) {
          this.transportFailure(new CursorCliError('Cursor Agent ACP frame exceeded the limit'));
          return;
        }
        const end = line.at(-1) === 0x0d ? line.length - 1 : line.length;
        this.receive(line.subarray(0, end).toString('utf8'));
      }
      if (buffered.length > this.maxFrameBytes) {
        this.transportFailure(new CursorCliError('Cursor Agent ACP frame exceeded the limit'));
      }
    });
    child.stdout.on('error', () => {
      this.transportFailure(new CursorCliError('Cursor Agent ACP output stream failed'));
    });
    child.once('error', () => {
      this.transportFailure(new CursorCliError('Cursor Agent ACP process failed to start'));
    });
    child.once('exit', (code, signal) => {
      this.closed = true;
      this.rejectPending(
        new CursorCliError(
          `Cursor Agent ACP process exited before completing its request (${processExit(code, signal)})`,
        ),
      );
      this.finishProcess();
    });
    child.once('close', () => this.finishProcess());
  }

  async initialize(): Promise<void> {
    const response = InitializeResponseSchema.parse(
      await this.request(
        'initialize',
        {
          protocolVersion: 1,
          clientCapabilities: {
            fs: { readTextFile: false, writeTextFile: false },
            terminal: false,
            _meta: { parameterizedModelPicker: true },
          },
          clientInfo: { name: 'iii-cursor-worker', version: '0.1.0' },
        },
        this.startupTimeoutMs,
      ),
    );
    if (!response.authMethods?.some((method) => method.id === 'cursor_login')) {
      throw new CursorCliError('Cursor Agent ACP does not advertise cursor_login authentication');
    }
  }

  async newSession(cwd: string): Promise<CursorAcpSession> {
    const response = NewSessionResponseSchema.parse(
      await this.request('session/new', { cwd, mcpServers: [] }),
    );
    return sessionDetails(response.sessionId, response.models);
  }

  async loadSession(sessionId: string, cwd: string): Promise<CursorAcpSession> {
    const response = LoadSessionResponseSchema.parse(
      await this.request('session/load', { sessionId, cwd, mcpServers: [] }),
    );
    return sessionDetails(sessionId, response.models);
  }

  async setModel(sessionId: string, model: string): Promise<void> {
    await this.request('session/set_config_option', {
      sessionId,
      configId: 'model',
      value: model,
    });
  }

  async setMode(sessionId: string, mode: 'agent' | 'plan' | 'ask'): Promise<void> {
    await this.request('session/set_mode', { sessionId, modeId: mode });
  }

  async prompt(
    sessionId: string,
    prompt: string,
    onUpdate: (update: CursorAcpSessionUpdate) => Promise<void>,
  ): Promise<'end_turn' | 'max_tokens' | 'max_turn_requests' | 'refusal' | 'cancelled'> {
    if (this.activePromptSessionId) {
      throw new CursorCliError('Cursor Agent ACP already has an active prompt');
    }
    this.activePromptSessionId = sessionId;
    this.updateHandler = onUpdate;
    this.updateQueue = Promise.resolve();
    this.updateError = null;
    try {
      const result = PromptResponseSchema.parse(
        await this.request(
          'session/prompt',
          { sessionId, prompt: [{ type: 'text', text: prompt }] },
          LONG_TURN_TIMEOUT_MS,
        ),
      );
      await new Promise<void>((resolvePromise) => setImmediate(resolvePromise));
      await this.updateQueue;
      if (this.updateError) throw this.updateError;
      return result.stopReason;
    } finally {
      this.updateHandler = null;
      this.activePromptSessionId = null;
    }
  }

  async cancel(sessionId: string): Promise<void> {
    await this.writeNotification({
      jsonrpc: '2.0',
      method: 'session/cancel',
      params: { sessionId },
    });
  }

  async close(): Promise<void> {
    if (this.processFinished) return;
    const startedAtMs = Date.now();
    this.closed = true;
    this.child.stdin.end();
    if (this.child.exitCode === null) this.child.kill('SIGTERM');
    this.rejectPending(new CursorCliError('Cursor Agent ACP client closed'));
    await waitForProcess(this.processClosed, Math.max(1, Math.floor(this.shutdownTimeoutMs / 2)));
    if (!this.processFinished && this.child.exitCode === null) {
      this.child.kill('SIGKILL');
      const remainingMs = Math.max(0, this.shutdownTimeoutMs - (Date.now() - startedAtMs));
      await waitForProcess(this.processClosed, remainingMs);
    }
  }

  forceClose(): void {
    if (this.child.exitCode === null) this.child.kill('SIGKILL');
    this.closed = true;
    this.rejectPending(new CursorCliError('Cursor Agent ACP client closed'));
  }

  private request(
    method: string,
    params: JsonObject,
    timeoutMs = this.rpcTimeoutMs,
  ): Promise<unknown> {
    if (this.closed || !this.child.stdin.writable) {
      return Promise.reject(new CursorCliError('Cursor Agent ACP process is not available'));
    }
    const id = this.nextId;
    this.nextId += 1;
    return new Promise((resolvePromise, rejectPromise) => {
      const timer = setTimeout(() => {
        this.pending.delete(id);
        rejectPromise(new CursorCliError(`Cursor Agent ACP ${method} timed out`));
      }, timeoutMs);
      this.pending.set(id, { resolve: resolvePromise, reject: rejectPromise, timer });
      this.write({ jsonrpc: '2.0', id, method, params });
    });
  }

  private receive(line: string): void {
    let message: JsonObject;
    try {
      const parsed: unknown = JSON.parse(line);
      if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed)) return;
      message = parsed as JsonObject;
    } catch {
      return;
    }
    if (message.jsonrpc !== '2.0') return;
    const hasId = Object.hasOwn(message, 'id');
    const id =
      typeof message.id === 'number' || typeof message.id === 'string' || message.id === null
        ? message.id
        : undefined;
    const method = typeof message.method === 'string' ? message.method : null;
    if (method && hasId && id !== undefined) {
      this.handleReverseRequest(id, method);
      return;
    }
    if (method === 'session/update') {
      const update = parseSessionUpdate(message.params);
      if (update && this.updateHandler && update.sessionId === this.activePromptSessionId) {
        const handler = this.updateHandler;
        this.updateQueue = this.updateQueue.then(async () => {
          if (this.updateError) return;
          try {
            await handler(update);
          } catch (error) {
            this.updateError = error instanceof Error ? error : new Error(String(error));
          }
        });
      }
      return;
    }
    if (id === undefined || id === null) return;
    const pending = this.pending.get(id);
    if (!pending) return;
    this.pending.delete(id);
    clearTimeout(pending.timer);
    if (message.error && typeof message.error === 'object') {
      const error = message.error as JsonObject;
      pending.reject(
        new CursorAcpRpcError(
          typeof error.code === 'number' ? error.code : -32000,
          safeRemoteMessage(error.message),
        ),
      );
      return;
    }
    pending.resolve(message.result);
  }

  private handleReverseRequest(id: number | string | null, method: string): void {
    if (method === 'session/request_permission') {
      this.write({
        jsonrpc: '2.0',
        id,
        result: { outcome: { outcome: 'cancelled' } },
      });
      return;
    }
    this.write({
      jsonrpc: '2.0',
      id,
      error: { code: -32601, message: 'Client capability is unavailable' },
    });
  }

  private write(message: JsonObject): void {
    try {
      this.child.stdin.write(`${JSON.stringify(message)}\n`, (error) => {
        if (error) {
          this.transportFailure(new CursorCliError('Cursor Agent ACP input stream failed'));
        }
      });
    } catch {
      this.transportFailure(new CursorCliError('Cursor Agent ACP input stream failed'));
    }
  }

  private writeNotification(message: JsonObject): Promise<void> {
    if (this.closed || !this.child.stdin.writable) {
      return Promise.reject(new CursorCliError('Cursor Agent ACP process is not available'));
    }
    return new Promise((resolvePromise, rejectPromise) => {
      try {
        this.child.stdin.write(`${JSON.stringify(message)}\n`, (error) => {
          if (error) {
            const failure = new CursorCliError('Cursor Agent ACP input stream failed');
            this.transportFailure(failure);
            rejectPromise(failure);
            return;
          }
          resolvePromise();
        });
      } catch {
        const failure = new CursorCliError('Cursor Agent ACP input stream failed');
        this.transportFailure(failure);
        rejectPromise(failure);
      }
    });
  }

  private rejectPending(error: Error): void {
    for (const pending of this.pending.values()) {
      clearTimeout(pending.timer);
      pending.reject(error);
    }
    this.pending.clear();
  }

  private transportFailure(error: Error): void {
    if (this.closed) return;
    this.closed = true;
    this.rejectPending(error);
    if (this.child.exitCode === null) this.child.kill('SIGTERM');
  }

  private finishProcess(): void {
    if (this.processFinished) return;
    this.processFinished = true;
    this.closed = true;
    this.resolveProcessClosed();
    if (!this.closeReported) {
      this.closeReported = true;
      this.onClose();
    }
  }
}

export class CursorCliError extends Error {
  constructor(message: string) {
    super(message);
    this.name = 'CursorCliError';
  }
}

export class CursorAcpRpcError extends CursorCliError {
  constructor(
    readonly code: number,
    message: string,
  ) {
    super(`Cursor Agent ACP request failed (${code}): ${message}`);
    this.name = 'CursorAcpRpcError';
  }
}

async function validateCursorAgent(
  binary: string,
  dependencies: Pick<CursorCliDependencies, 'run' | 'env'> & { cwd?: string },
  timeoutMs: number,
): Promise<CursorCliExecutable | null> {
  try {
    const versionResult = await dependencies.run(binary, ['--version'], {
      cwd: dependencies.cwd,
      env: loginEnvironment(dependencies.env),
      timeoutMs,
    });
    const version = versionResult.stdout.trim().split(/\s+/)[0] ?? '';
    if (!CURSOR_VERSION.test(version)) return null;
    const helpResult = await dependencies.run(binary, ['acp', '--help'], {
      cwd: dependencies.cwd,
      env: loginEnvironment(dependencies.env),
      timeoutMs,
    });
    if (!helpResult.stdout.includes('Cursor Agent') || !helpResult.stdout.includes('ACP')) {
      return null;
    }
    return { binary, version };
  } catch {
    return null;
  }
}

function cursorAgentCandidates(
  configured: string,
  dependencies: Pick<CursorCliDependencies, 'home' | 'env'>,
  discoveryCwd: string,
): string[] {
  if (configured) {
    return hasPathComponent(configured)
      ? [resolve(discoveryCwd, configured)]
      : pathCandidates(configured, dependencies.env, discoveryCwd, true);
  }
  return [
    join(dependencies.home, '.local', 'bin', 'cursor-agent'),
    join(dependencies.home, '.local', 'bin', 'agent'),
    ...pathCandidates('cursor-agent', dependencies.env, discoveryCwd),
  ].filter((candidate, index, candidates) => candidates.indexOf(candidate) === index);
}

function pathCandidates(
  name: string,
  environment: NodeJS.ProcessEnv,
  discoveryCwd: string,
  allowRelativeDirectories = false,
): string[] {
  const path = environment.PATH;
  if (!path) return [];
  return path
    .split(delimiter)
    .filter(
      (directory) => directory.length > 0 && (allowRelativeDirectories || isAbsolute(directory)),
    )
    .map((directory) => resolve(discoveryCwd, directory, name))
    .filter((candidate, index, candidates) => candidates.indexOf(candidate) === index);
}

function hasPathComponent(binary: string): boolean {
  return isAbsolute(binary) || binary.includes('/') || binary.includes('\\');
}

async function identityCanonicalPath(path: string): Promise<string> {
  return path;
}

function sessionDetails(
  sessionId: string,
  models: z.infer<typeof ModelStateSchema>,
): CursorAcpSession {
  return {
    sessionId,
    currentModelId: models?.currentModelId ?? null,
    models: (models?.availableModels ?? []).map((model) => ({
      modelId: model.modelId,
      name: model.name ?? model.modelId,
    })),
  };
}

function publicCursorAcpModels(models: CursorAcpSession['models']): CursorModel[] {
  const catalog = new Map<string, CursorModel>();
  for (const model of models) {
    const id = model.modelId === ACP_AUTO_MODEL_ID ? PUBLIC_AUTO_MODEL_ID : model.modelId;
    if (!id || catalog.has(id)) continue;
    catalog.set(id, {
      id,
      display_name: model.name,
      description: '',
      parameters: [],
      variants: [],
    });
  }
  return [...catalog.values()];
}

function parseSessionUpdate(value: unknown): CursorAcpSessionUpdate | null {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return null;
  const params = value as JsonObject;
  if (typeof params.sessionId !== 'string') return null;
  if (!params.update || typeof params.update !== 'object' || Array.isArray(params.update)) {
    return null;
  }
  return { sessionId: params.sessionId, update: params.update as JsonObject };
}

function loginEnvironment(source: NodeJS.ProcessEnv): NodeJS.ProcessEnv {
  const environment: NodeJS.ProcessEnv = {};
  const allowed = new Set([
    'HOME',
    'PATH',
    'USER',
    'LOGNAME',
    'LANG',
    'LC_ALL',
    'TERM',
    'TMPDIR',
    'SHELL',
    'XDG_CONFIG_HOME',
    'XDG_CACHE_HOME',
    'XDG_DATA_HOME',
  ]);
  for (const [key, value] of Object.entries(source)) {
    if (!allowed.has(key) || value === undefined) continue;
    if (key === 'PATH') {
      const safePath = value
        .split(delimiter)
        .filter((directory) => directory.length > 0 && isAbsolute(directory))
        .join(delimiter);
      if (safePath) environment.PATH = safePath;
      continue;
    }
    environment[key] = value;
  }
  return environment;
}

function safeRemoteMessage(value: unknown): string {
  const message = typeof value === 'string' ? value : 'request failed';
  const withoutControlCharacters = Array.from(message, (character) => {
    const codePoint = character.codePointAt(0) ?? 0;
    return codePoint <= 31 || codePoint === 127 ? ' ' : character;
  }).join('');
  return withoutControlCharacters
    .replace(
      /(["']?(?:access_?token|refresh_?token|api_?key|token|key)["']?\s*[:=]\s*["']?)[^\s"',}]+/gi,
      '$1[redacted]',
    )
    .replace(/\bBearer\s+[A-Za-z0-9._~+/=-]+/gi, 'Bearer [redacted]')
    .replace(/\beyJ[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\b/g, '[redacted]')
    .replace(/\b(?:cursor|access|refresh)[-_][A-Za-z0-9._-]{16,}\b/gi, '[redacted]')
    .replace(/[A-Z0-9._%+-]+@[A-Z0-9.-]+\.[A-Z]{2,}/gi, '[redacted-email]')
    .replace(/\b(?:Bearer\s+)?(?:key_|sk-)[A-Za-z0-9._-]+\b/gi, '[redacted]')
    .slice(0, 500);
}

function processExit(code: number | null, signal: NodeJS.Signals | null): string {
  if (signal) return `signal ${signal}`;
  return `code ${code ?? 'unknown'}`;
}

async function waitForProcess(completion: Promise<void>, timeoutMs: number): Promise<void> {
  let timer: ReturnType<typeof setTimeout> | undefined;
  await Promise.race([
    completion,
    new Promise<void>((resolvePromise) => {
      timer = setTimeout(resolvePromise, timeoutMs);
    }),
  ]);
  if (timer) clearTimeout(timer);
}

function runCommand(
  command: string,
  args: string[],
  options: { cwd?: string; env: NodeJS.ProcessEnv; timeoutMs: number },
): Promise<CommandResult> {
  return new Promise((resolvePromise, rejectPromise) => {
    nodeExecFile(
      command,
      args,
      {
        cwd: options.cwd,
        env: options.env,
        encoding: 'utf8',
        maxBuffer: 1024 * 1024,
        timeout: options.timeoutMs,
      },
      (error, stdout, stderr) => {
        if (error) {
          rejectPromise(new CursorCliError(`Cursor Agent command failed: ${basename(command)}`));
          return;
        }
        resolvePromise({ stdout, stderr });
      },
    );
  });
}
