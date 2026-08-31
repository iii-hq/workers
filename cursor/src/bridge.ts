import { spawn as nodeSpawn } from 'node:child_process';
import { readFile as nodeReadFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { createInterface } from 'node:readline';
import { z } from 'zod';

const READY_PREFIX = 'cursor-sdk-bridge ready ';
const REQUIRED_CAPABILITIES = [
  'agent.create',
  'agent.resume',
  'agent.send',
  'run.observe',
  'run.wait',
  'run.cancel',
  'agent.management',
  'cursor.catalog',
  'agent.usage',
];

const DiscoverySchema = z
  .object({
    schemaVersion: z.literal(1),
    serverVersion: z.string().optional(),
    pid: z.number().int().optional(),
    transport: z.literal('tcp'),
    protocol: z.literal('connect'),
    host: z.string().optional(),
    port: z.number().int().positive().optional(),
    url: z.string().optional(),
    authTokenFile: z.string().optional(),
    authToken: z.string().optional(),
    maxMessageBytes: z.number().int().positive().optional(),
  })
  .passthrough();

const PingSchema = z.object({ message: z.string() }).passthrough();
const VersionSchema = z
  .object({
    bridgeVersion: z.string(),
    protocolVersion: z.string(),
    capabilities: z.array(z.string()).default([]),
  })
  .passthrough();

type JsonObject = Record<string, unknown>;

export type BridgeLaunchOptions = {
  binary: string;
  workspace: string;
  apiKey: string;
  startupTimeoutMs: number;
  shutdownTimeoutMs: number;
  rpcTimeoutMs: number;
  maxFrameBytes: number;
};

export type RpcOptions = {
  signal?: AbortSignal;
  timeoutMs?: number;
};

export interface BridgeClient {
  unary<T>(
    service: string,
    method: string,
    request: JsonObject,
    responseSchema: z.ZodType<T>,
    options?: RpcOptions,
  ): Promise<T>;
  stream<T>(
    service: string,
    method: string,
    request: JsonObject,
    responseSchema: z.ZodType<T>,
    options?: RpcOptions,
  ): AsyncIterable<T>;
  close(): Promise<void>;
}

export interface BridgeClientFactory {
  create(options: BridgeLaunchOptions): BridgeClient;
  closeAll(): Promise<void>;
  forceCloseAll(): void;
}

export type BridgeProcess = {
  stderr: NodeJS.ReadableStream;
  exitCode: number | null;
  pid?: number;
  once(event: string, listener: (...args: unknown[]) => void): BridgeProcess;
  off(event: string, listener: (...args: unknown[]) => void): BridgeProcess;
  kill(signal?: NodeJS.Signals | number): boolean;
};

export type ProcessSpawner = (
  command: string,
  args: string[],
  options: { env: NodeJS.ProcessEnv; stdio: ['ignore', 'ignore', 'pipe'] },
) => BridgeProcess;

export type BridgeDependencies = {
  spawn: ProcessSpawner;
  readFile: (path: string, encoding: BufferEncoding) => Promise<string>;
  fetch: typeof fetch;
};

export type SdkErrorDetail = {
  request_id?: string;
  sdk_error_code?: string | number;
  message?: string;
  help_url?: string;
  provider?: string;
  retry_after_seconds?: number;
  rate_limit?: {
    limit?: string;
    remaining?: string;
    reset_epoch_seconds?: string;
  };
};

export class BridgeProcessError extends Error {
  constructor(message: string) {
    super(message);
    this.name = 'BridgeProcessError';
  }
}

export class BridgeTransportError extends Error {
  constructor(message: string) {
    super(message);
    this.name = 'BridgeTransportError';
  }
}

export class BridgeRpcError extends Error {
  readonly code: string;
  readonly detail: SdkErrorDetail | null;
  readonly rawDetails: unknown[];

  constructor(code: string, message: string, detail: SdkErrorDetail | null, rawDetails: unknown[]) {
    super(`${code}: ${detail?.message || message}`);
    this.name = 'BridgeRpcError';
    this.code = code;
    this.detail = detail;
    this.rawDetails = rawDetails;
  }
}

export class ConnectJsonTransport {
  constructor(
    private readonly baseUrl: string,
    private readonly token: string,
    private readonly fetchImpl: typeof fetch,
    private readonly defaultTimeoutMs: number,
    private readonly maxFrameBytes: number,
  ) {}

  async unary<T>(
    service: string,
    method: string,
    request: JsonObject,
    responseSchema: z.ZodType<T>,
    options: RpcOptions = {},
  ): Promise<T> {
    const requestBody = Buffer.from(JSON.stringify(request));
    if (requestBody.length > this.maxFrameBytes) {
      throw new BridgeTransportError(`Connect request exceeds ${this.maxFrameBytes} bytes`);
    }
    const response = await this.post(
      service,
      method,
      'application/json',
      requestBody,
      withTimeout(options.signal, options.timeoutMs ?? this.defaultTimeoutMs),
    );
    if (!response.ok) throw await rpcErrorFromResponse(response, this.maxFrameBytes);
    const payload = await readJsonResponse(response, this.maxFrameBytes);
    return responseSchema.parse(payload);
  }

  async *stream<T>(
    service: string,
    method: string,
    request: JsonObject,
    responseSchema: z.ZodType<T>,
    options: RpcOptions = {},
  ): AsyncIterable<T> {
    const payload = Buffer.from(JSON.stringify(request));
    if (payload.length > this.maxFrameBytes) {
      throw new BridgeTransportError(`Connect request frame exceeds ${this.maxFrameBytes} bytes`);
    }
    const frame = Buffer.allocUnsafe(5 + payload.length);
    frame.writeUInt8(0, 0);
    frame.writeUInt32BE(payload.length, 1);
    payload.copy(frame, 5);
    const response = await this.post(
      service,
      method,
      'application/connect+json',
      frame,
      options.signal,
    );
    if (!response.ok) throw await rpcErrorFromResponse(response, this.maxFrameBytes);
    if (!response.body) throw new BridgeTransportError('Connect stream has no response body');

    const reader = response.body.getReader();
    let buffered = Buffer.alloc(0);
    let ended = false;
    try {
      while (!ended) {
        const read = await readWithIdleTimeout(reader, options.timeoutMs ?? this.defaultTimeoutMs);
        if (read.done) break;
        buffered = Buffer.concat([buffered, Buffer.from(read.value)]);
        while (buffered.length >= 5) {
          const flags = buffered.readUInt8(0);
          const length = buffered.readUInt32BE(1);
          if (length > this.maxFrameBytes) {
            throw new BridgeTransportError(
              `Connect response frame exceeds ${this.maxFrameBytes} bytes`,
            );
          }
          if (buffered.length < 5 + length) break;
          const body = buffered.subarray(5, 5 + length);
          buffered = buffered.subarray(5 + length);
          if ((flags & 0x01) !== 0) {
            throw new BridgeTransportError('Compressed Connect frames are not supported');
          }
          if ((flags & 0x02) !== 0) {
            const end = parseJsonObject(body);
            if (end.error) throw rpcErrorFromBody(end.error);
            ended = true;
            break;
          }
          if (flags !== 0) {
            throw new BridgeTransportError(`Unsupported Connect frame flags: ${flags}`);
          }
          yield responseSchema.parse(parseJsonObject(body));
        }
      }
    } catch (error) {
      if (error instanceof BridgeRpcError || error instanceof BridgeTransportError) throw error;
      if (options.signal?.aborted) throw options.signal.reason;
      throw new BridgeTransportError(`Connect stream failed: ${errorText(error)}`);
    } finally {
      await reader.cancel().catch(() => undefined);
    }
    if (!ended) {
      throw new BridgeTransportError('Connect stream ended before EndStreamResponse');
    }
  }

  private post(
    service: string,
    method: string,
    contentType: string,
    body: Uint8Array,
    signal?: AbortSignal,
  ): Promise<Response> {
    return this.fetchImpl(`${this.baseUrl}/sdk.v1.${service}/${method}`, {
      method: 'POST',
      redirect: 'error',
      headers: {
        Authorization: `Bearer ${this.token}`,
        'Connect-Protocol-Version': '1',
        'Content-Type': contentType,
      },
      body: body as unknown as BodyInit,
      signal,
    });
  }
}

export class ManagedBridgeClient implements BridgeClient {
  private process: BridgeProcess | null = null;
  private transport: ConnectJsonTransport | null = null;
  private startPromise: Promise<ConnectJsonTransport> | null = null;
  private closePromise: Promise<void> | null = null;

  constructor(
    private readonly options: BridgeLaunchOptions,
    private readonly dependencies: BridgeDependencies = defaultDependencies(),
    private readonly onClosed: () => void = () => undefined,
  ) {}

  async unary<T>(
    service: string,
    method: string,
    request: JsonObject,
    responseSchema: z.ZodType<T>,
    options?: RpcOptions,
  ): Promise<T> {
    const transport = await this.ensureStarted();
    return transport.unary(service, method, request, responseSchema, options);
  }

  async *stream<T>(
    service: string,
    method: string,
    request: JsonObject,
    responseSchema: z.ZodType<T>,
    options?: RpcOptions,
  ): AsyncIterable<T> {
    const transport = await this.ensureStarted();
    yield* transport.stream(service, method, request, responseSchema, options);
  }

  close(): Promise<void> {
    if (!this.closePromise) this.closePromise = this.closeInner();
    return this.closePromise;
  }

  forceClose(): void {
    if (this.process?.exitCode === null) this.process.kill('SIGTERM');
  }

  private async ensureStarted(): Promise<ConnectJsonTransport> {
    if (this.closePromise) throw new BridgeProcessError('Cursor SDK Bridge client is closed');
    if (!this.startPromise) {
      const pending = this.start();
      this.startPromise = pending;
      void pending.catch(() => {
        if (this.startPromise === pending) this.startPromise = null;
      });
    }
    return this.startPromise;
  }

  private async start(): Promise<ConnectJsonTransport> {
    const binary =
      this.options.binary.trim() ||
      process.env.CURSOR_SDK_BRIDGE_BIN?.trim() ||
      'cursor-sdk-bridge';
    const workspace = resolve(this.options.workspace || '.');
    const env: NodeJS.ProcessEnv = {
      ...process.env,
      CURSOR_SDK_CLIENT_LANGUAGE: 'node',
    };
    if (this.options.apiKey.trim()) env.CURSOR_API_KEY = this.options.apiKey.trim();
    let child: BridgeProcess;
    try {
      child = this.dependencies.spawn(binary, ['--workspace', workspace], {
        env,
        stdio: ['ignore', 'ignore', 'pipe'],
      });
    } catch (error) {
      throw new BridgeProcessError(
        `Could not launch Cursor SDK Bridge binary ${JSON.stringify(binary)}: ${errorText(error)}`,
      );
    }
    this.process = child;
    try {
      const discovery = await awaitDiscovery(
        child,
        this.options.startupTimeoutMs,
        this.options.apiKey,
      );
      const baseUrl = discoveryUrl(discovery);
      assertLoopback(baseUrl);
      const token = discovery.authToken?.trim()
        ? discovery.authToken.trim()
        : await readAuthToken(discovery.authTokenFile, this.dependencies.readFile);
      const maxFrameBytes = Math.min(
        this.options.maxFrameBytes,
        discovery.maxMessageBytes ?? this.options.maxFrameBytes,
      );
      const transport = new ConnectJsonTransport(
        baseUrl,
        token,
        this.dependencies.fetch,
        this.options.rpcTimeoutMs,
        maxFrameBytes,
      );
      const ping = await transport.unary('SdkBridgeControlService', 'Ping', {}, PingSchema);
      if (ping.message !== 'pong') throw new BridgeProcessError('Cursor SDK Bridge Ping failed');
      const version = await transport.unary(
        'SdkBridgeControlService',
        'GetVersion',
        {},
        VersionSchema,
      );
      if (version.protocolVersion !== 'sdk.v1') {
        throw new BridgeProcessError(
          `Cursor SDK Bridge protocol ${JSON.stringify(version.protocolVersion)} is not sdk.v1`,
        );
      }
      const missing = REQUIRED_CAPABILITIES.filter(
        (capability) => !version.capabilities.includes(capability),
      );
      if (missing.length > 0) {
        throw new BridgeProcessError(
          `Cursor SDK Bridge is missing required capabilities: ${missing.join(', ')}`,
        );
      }
      this.transport = transport;
      return transport;
    } catch (error) {
      await stopProcess(child, this.options.shutdownTimeoutMs, false);
      this.process = null;
      throw error;
    }
  }

  private async closeInner(): Promise<void> {
    let gracefulTerminationRequested = false;
    try {
      const transport = this.transport ?? (await this.startPromise?.catch(() => null));
      if (transport && this.process?.exitCode === null) {
        gracefulTerminationRequested = true;
        await transport
          .unary(
            'SdkBridgeControlService',
            'Shutdown',
            { graceSeconds: 2 },
            z.object({}).passthrough(),
            { timeoutMs: this.options.shutdownTimeoutMs },
          )
          .catch(() => undefined);
      }
      if (this.process) {
        await stopProcess(
          this.process,
          this.options.shutdownTimeoutMs,
          gracefulTerminationRequested,
        );
      }
    } finally {
      this.process = null;
      this.transport = null;
      this.onClosed();
    }
  }
}

export class ProductionBridgeClientFactory implements BridgeClientFactory {
  private readonly clients = new Set<ManagedBridgeClient>();

  constructor(private readonly dependencies: BridgeDependencies = defaultDependencies()) {}

  create(options: BridgeLaunchOptions): BridgeClient {
    let client: ManagedBridgeClient;
    client = new ManagedBridgeClient(options, this.dependencies, () => this.clients.delete(client));
    this.clients.add(client);
    return client;
  }

  async closeAll(): Promise<void> {
    await Promise.all([...this.clients].map((client) => client.close()));
  }

  forceCloseAll(): void {
    for (const client of this.clients) client.forceClose();
  }
}

export function parseDiscoveryLine(line: string): z.infer<typeof DiscoverySchema> | null {
  if (!line.startsWith(READY_PREFIX)) return null;
  let parsed: unknown;
  try {
    parsed = JSON.parse(line.slice(READY_PREFIX.length));
  } catch (error) {
    throw new BridgeProcessError(
      `Cursor SDK Bridge discovery JSON is invalid: ${errorText(error)}`,
    );
  }
  return DiscoverySchema.parse(parsed);
}

export function decodeSdkErrorDetail(value: string): SdkErrorDetail | null {
  try {
    const bytes = Buffer.from(value, 'base64url');
    const fields = decodeMessage(bytes);
    const code = numericField(fields, 2);
    const retry = bytesField(fields, 6);
    const limit = bytesField(fields, 7);
    const detail: SdkErrorDetail = {
      request_id: stringField(fields, 1),
      sdk_error_code:
        code === undefined ? undefined : (SDK_ERROR_CODES[Number(code)] ?? Number(code)),
      message: stringField(fields, 3),
      help_url: stringField(fields, 4),
      provider: stringField(fields, 5),
    };
    if (retry) {
      const duration = decodeMessage(retry);
      const seconds = numericField(duration, 1) ?? 0n;
      const nanos = numericField(duration, 2) ?? 0n;
      detail.retry_after_seconds = Number(seconds) + Number(nanos) / 1_000_000_000;
    }
    if (limit) {
      const values = decodeMessage(limit);
      detail.rate_limit = {
        limit: bigintString(numericField(values, 1)),
        remaining: bigintString(numericField(values, 2)),
        reset_epoch_seconds: bigintString(numericField(values, 3)),
      };
    }
    return detail;
  } catch {
    return null;
  }
}

function defaultDependencies(): BridgeDependencies {
  return {
    spawn: (command, args, options) =>
      nodeSpawn(command, args, options) as unknown as BridgeProcess,
    readFile: nodeReadFile,
    fetch,
  };
}

async function awaitDiscovery(
  child: BridgeProcess,
  timeoutMs: number,
  apiKey: string,
): Promise<z.infer<typeof DiscoverySchema>> {
  return new Promise((resolvePromise, rejectPromise) => {
    const diagnostics: string[] = [];
    let settled = false;
    const lines = createInterface({ input: child.stderr });
    const settle = (
      result: { ok: true; value: z.infer<typeof DiscoverySchema> } | { ok: false; error: Error },
    ) => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      child.off('exit', onExit);
      child.off('error', onError);
      lines.close();
      if (result.ok) resolvePromise(result.value);
      else rejectPromise(result.error);
    };
    const onExit = (code: unknown, signal: unknown) => {
      const context = diagnostics.length > 0 ? `: ${diagnostics.join('\n')}` : '';
      settle({
        ok: false,
        error: new BridgeProcessError(
          `Cursor SDK Bridge exited before ready (code=${String(code)}, signal=${String(signal)})${context}`,
        ),
      });
    };
    const onError = (error: unknown) =>
      settle({
        ok: false,
        error: new BridgeProcessError(`Cursor SDK Bridge process failed: ${errorText(error)}`),
      });
    const timer = setTimeout(
      () =>
        settle({
          ok: false,
          error: new BridgeProcessError(
            `Timed out after ${timeoutMs}ms waiting for Cursor SDK Bridge`,
          ),
        }),
      timeoutMs,
    );
    child.once('exit', onExit);
    child.once('error', onError);
    lines.on('line', (line) => {
      if (settled) return;
      if (!line.startsWith(READY_PREFIX)) {
        if (diagnostics.length < 20) diagnostics.push(redact(line, apiKey));
        return;
      }
      try {
        const discovery = parseDiscoveryLine(line);
        if (!discovery) throw new BridgeProcessError('Cursor SDK Bridge ready line is invalid');
        settle({ ok: true, value: discovery });
      } catch (error) {
        settle({
          ok: false,
          error: redactedDiscoveryError(error, apiKey),
        });
      }
    });
  });
}

function discoveryUrl(discovery: z.infer<typeof DiscoverySchema>): string {
  if (discovery.url) return discovery.url.replace(/\/$/, '');
  if (!discovery.host || !discovery.port) {
    throw new BridgeProcessError('Cursor SDK Bridge discovery omitted URL and host/port');
  }
  const host = discovery.host.includes(':') ? `[${discovery.host}]` : discovery.host;
  return `http://${host}:${discovery.port}`;
}

function assertLoopback(rawUrl: string): void {
  let url: URL;
  try {
    url = new URL(rawUrl);
  } catch {
    throw new BridgeProcessError('Cursor SDK Bridge discovery URL is invalid');
  }
  const allowed = new Set(['127.0.0.1', 'localhost', '::1', '[::1]']);
  if (url.protocol !== 'http:' || !allowed.has(url.hostname)) {
    throw new BridgeProcessError('Cursor SDK Bridge must use a loopback HTTP endpoint');
  }
}

async function readAuthToken(
  tokenFile: string | undefined,
  readFile: BridgeDependencies['readFile'],
): Promise<string> {
  if (!tokenFile) throw new BridgeProcessError('Cursor SDK Bridge discovery omitted authTokenFile');
  let token: string;
  try {
    token = (await readFile(tokenFile, 'utf8')).trim();
  } catch (error) {
    throw new BridgeProcessError(
      `Could not read Cursor SDK Bridge auth token file: ${errorText(error)}`,
    );
  }
  if (!token) throw new BridgeProcessError('Cursor SDK Bridge auth token file is empty');
  return token;
}

async function stopProcess(
  child: BridgeProcess,
  timeoutMs: number,
  gracefulTerminationRequested: boolean,
): Promise<void> {
  if (child.exitCode !== null) return;
  if (gracefulTerminationRequested && (await waitForExit(child, timeoutMs))) return;
  child.kill('SIGTERM');
  if (await waitForExit(child, timeoutMs)) return;
  child.kill('SIGKILL');
  await waitForExit(child, timeoutMs);
}

function waitForExit(child: BridgeProcess, timeoutMs: number): Promise<boolean> {
  if (child.exitCode !== null) return Promise.resolve(true);
  return new Promise((resolvePromise) => {
    const onExit = () => {
      clearTimeout(timer);
      resolvePromise(true);
    };
    const timer = setTimeout(() => {
      child.off('exit', onExit);
      resolvePromise(false);
    }, timeoutMs);
    child.once('exit', onExit);
  });
}

function withTimeout(signal: AbortSignal | undefined, timeoutMs: number): AbortSignal {
  const timeout = AbortSignal.timeout(timeoutMs);
  return signal ? AbortSignal.any([signal, timeout]) : timeout;
}

async function rpcErrorFromResponse(response: Response, maxBytes: number): Promise<BridgeRpcError> {
  let body: unknown;
  try {
    body = await readJsonResponse(response, maxBytes);
  } catch {
    body = { code: `http_${response.status}`, message: response.statusText };
  }
  return rpcErrorFromBody(body);
}

function rpcErrorFromBody(input: unknown): BridgeRpcError {
  const body =
    input && typeof input === 'object' ? (input as Record<string, unknown>) : ({} as JsonObject);
  const code = typeof body.code === 'string' ? body.code : 'unknown';
  const message = typeof body.message === 'string' ? body.message : 'Cursor SDK Bridge RPC failed';
  const rawDetails = Array.isArray(body.details) ? body.details : [];
  let detail: SdkErrorDetail | null = null;
  for (const candidate of rawDetails) {
    if (!candidate || typeof candidate !== 'object') continue;
    const entry = candidate as Record<string, unknown>;
    const type = typeof entry.type === 'string' ? entry.type : '';
    if (!type.endsWith('sdk.v1.SdkErrorDetails') || typeof entry.value !== 'string') continue;
    detail = decodeSdkErrorDetail(entry.value);
    if (detail) break;
  }
  return new BridgeRpcError(code, message, detail, rawDetails);
}

function parseJsonObject(bytes: Uint8Array): JsonObject {
  let value: unknown;
  try {
    value = JSON.parse(Buffer.from(bytes).toString('utf8'));
  } catch (error) {
    throw new BridgeTransportError(`Connect frame is not valid JSON: ${errorText(error)}`);
  }
  if (!value || typeof value !== 'object' || Array.isArray(value)) {
    throw new BridgeTransportError('Connect frame JSON must be an object');
  }
  return value as JsonObject;
}

async function readJsonResponse(response: Response, maxBytes: number): Promise<JsonObject> {
  const declaredLength = Number(response.headers.get('content-length'));
  if (Number.isFinite(declaredLength) && declaredLength > maxBytes) {
    throw new BridgeTransportError(`Connect response exceeds ${maxBytes} bytes`);
  }
  if (!response.body) throw new BridgeTransportError('Connect response has no body');
  const reader = response.body.getReader();
  const chunks: Buffer[] = [];
  let length = 0;
  try {
    while (true) {
      const read = await reader.read();
      if (read.done) break;
      const chunk = Buffer.from(read.value);
      length += chunk.length;
      if (length > maxBytes) {
        throw new BridgeTransportError(`Connect response exceeds ${maxBytes} bytes`);
      }
      chunks.push(chunk);
    }
  } finally {
    await reader.cancel().catch(() => undefined);
  }
  return parseJsonObject(Buffer.concat(chunks, length));
}

async function readWithIdleTimeout(
  reader: ReadableStreamDefaultReader<Uint8Array>,
  timeoutMs: number,
): Promise<ReadableStreamReadResult<Uint8Array>> {
  let timer: ReturnType<typeof setTimeout> | undefined;
  try {
    return await Promise.race([
      reader.read(),
      new Promise<never>((_resolvePromise, rejectPromise) => {
        timer = setTimeout(
          () =>
            rejectPromise(
              new BridgeTransportError(`Connect stream idle timeout after ${timeoutMs}ms`),
            ),
          timeoutMs,
        );
      }),
    ]);
  } finally {
    if (timer) clearTimeout(timer);
  }
}

type DecodedField = { wire: number; value: bigint | Buffer };
type DecodedMessage = Map<number, DecodedField[]>;

function decodeMessage(bytes: Buffer): DecodedMessage {
  const fields: DecodedMessage = new Map();
  let offset = 0;
  while (offset < bytes.length) {
    const key = readVarint(bytes, offset);
    offset = key.offset;
    const number = Number(key.value >> 3n);
    const wire = Number(key.value & 7n);
    let value: bigint | Buffer;
    if (wire === 0) {
      const decoded = readVarint(bytes, offset);
      offset = decoded.offset;
      value = decoded.value;
    } else if (wire === 1) {
      if (offset + 8 > bytes.length) throw new Error('truncated fixed64');
      value = bytes.subarray(offset, offset + 8);
      offset += 8;
    } else if (wire === 2) {
      const decoded = readVarint(bytes, offset);
      offset = decoded.offset;
      const length = Number(decoded.value);
      if (!Number.isSafeInteger(length) || offset + length > bytes.length) {
        throw new Error('truncated length-delimited field');
      }
      value = bytes.subarray(offset, offset + length);
      offset += length;
    } else if (wire === 5) {
      if (offset + 4 > bytes.length) throw new Error('truncated fixed32');
      value = bytes.subarray(offset, offset + 4);
      offset += 4;
    } else {
      throw new Error(`unsupported protobuf wire type ${wire}`);
    }
    const list = fields.get(number) ?? [];
    list.push({ wire, value });
    fields.set(number, list);
  }
  return fields;
}

function readVarint(bytes: Buffer, start: number): { value: bigint; offset: number } {
  let value = 0n;
  let shift = 0n;
  let offset = start;
  while (offset < bytes.length && shift <= 63n) {
    const byte = bytes[offset];
    offset += 1;
    value |= BigInt(byte & 0x7f) << shift;
    if ((byte & 0x80) === 0) return { value, offset };
    shift += 7n;
  }
  throw new Error('invalid protobuf varint');
}

function numericField(fields: DecodedMessage, field: number): bigint | undefined {
  const entry = fields.get(field)?.[0];
  return entry?.wire === 0 && typeof entry.value === 'bigint' ? entry.value : undefined;
}

function bytesField(fields: DecodedMessage, field: number): Buffer | undefined {
  const entry = fields.get(field)?.[0];
  return entry?.wire === 2 && Buffer.isBuffer(entry.value) ? entry.value : undefined;
}

function stringField(fields: DecodedMessage, field: number): string | undefined {
  return bytesField(fields, field)?.toString('utf8');
}

function bigintString(value: bigint | undefined): string | undefined {
  return value === undefined ? undefined : value.toString();
}

function redact(value: string, apiKey: string): string {
  let redacted = value.replace(/(?:key|token|secret)_[A-Za-z0-9._-]+/gi, '[redacted]');
  if (apiKey) redacted = redacted.replaceAll(apiKey, '[redacted]');
  return redacted.slice(0, 500);
}

function redactedDiscoveryError(error: unknown, apiKey: string): Error {
  if (!(error instanceof Error)) {
    return new BridgeProcessError(
      `Cursor SDK Bridge discovery failed: ${redact(String(error), apiKey)}`,
    );
  }
  error.message = redact(error.message, apiKey);
  if (error.stack) error.stack = redact(error.stack, apiKey);
  return error;
}

function errorText(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

const SDK_ERROR_CODES: Record<number, string> = {
  0: 'UNSPECIFIED',
  1: 'UNAUTHORIZED',
  2: 'API_KEY_NOT_FOUND',
  3: 'PLAN_REQUIRED',
  4: 'ROLE_FORBIDDEN',
  5: 'FEATURE_UNAVAILABLE',
  6: 'AGENT_NOT_FOUND',
  7: 'RUN_NOT_FOUND',
  8: 'VALIDATION_ERROR',
  9: 'INVALID_MODEL',
  10: 'INVALID_BRANCH_NAME',
  11: 'REPOSITORY_REQUIRED',
  12: 'REPOSITORY_ACCESS',
  13: 'PR_RESOLUTION_FAILED',
  14: 'USAGE_LIMIT_EXCEEDED',
  15: 'AGENT_BUSY',
  16: 'AGENT_ARCHIVED',
  17: 'RUN_NOT_CANCELLABLE',
  18: 'RATE_LIMIT_EXCEEDED',
  19: 'UPSTREAM_ERROR',
  20: 'INTERNAL_ERROR',
  21: 'CLIENT_CANCELLED',
};
