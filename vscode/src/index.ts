import { execFile, spawn } from 'node:child_process';
import { watch } from 'node:fs';
import { mkdir, readFile, rm, stat } from 'node:fs/promises';
import { homedir } from 'node:os';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { promisify } from 'node:util';
import { registerWorker } from 'iii-sdk';
import { uiPage, uiStyles } from 'virtual:vscode-ui';
import {
  type Instance,
  instanceIdFor,
  isLoopback,
  publicInstance,
  schema,
  serverArgs,
  validateId,
  validateWorkspacePath,
} from './core.js';
import { findFreePort, processExited, stopProcess, waitForHttp } from './lifecycle.js';

const iii = registerWorker(process.env.III_URL ?? 'ws://127.0.0.1:49134', {
  workerName: 'vscode',
  workerDescription:
    'VS Code Workbench served by the VS Code Server CLI and presented as a Console page.',
});

const binary = process.env.VSCODE_SERVER_BIN ?? 'code';
const dataDir = process.env.VSCODE_DATA_DIR ?? join(homedir(), '.iii', 'vscode');
const bindHost = process.env.VSCODE_BIND_HOST ?? '127.0.0.1';
const portMin = Number(process.env.VSCODE_PORT_MIN ?? 18080);
const portMax = Number(process.env.VSCODE_PORT_MAX ?? 18180);
const startTimeoutMs = Number(process.env.VSCODE_START_TIMEOUT_MS ?? 180_000);
const stopGraceMs = 5_000;

if (!isLoopback(bindHost)) {
  console.error(`VSCODE_BIND_HOST must be a loopback address, got ${bindHost}`);
  process.exit(1);
}

const instances = new Map<string, Instance>();
const run = promisify(execFile);
let cliChecked = false;

const instanceFields = {
  id: { type: 'string' },
  name: { type: 'string' },
  workspace: { type: 'string' },
  host: { type: 'string' },
  port: { type: 'integer' },
  pid: { type: ['integer', 'null'] },
  started_at: { type: 'string' },
  status: { type: 'string', enum: ['starting', 'running', 'stopped', 'failed'] },
  exit_code: { type: ['integer', 'null'] },
};

async function ensureCli() {
  if (cliChecked) return;
  try {
    await run(binary, ['--version']);
  } catch {
    throw new Error(`VS Code CLI not available: ${binary}`);
  }
  cliChecked = true;
}

async function stop(instance: Instance) {
  await stopProcess(instance.process, { graceMs: stopGraceMs });
  instance.status = 'stopped';
}

async function waitUntilReady(instance: Instance) {
  const outcome = await waitForHttp({
    url: instance.url,
    timeoutMs: startTimeoutMs,
    exited: () => processExited(instance.process),
  });
  if (outcome === 'ready') {
    instance.status = 'running';
    return;
  }
  instance.status = 'failed';
  if (outcome === 'exited') {
    throw new Error(`VS Code Server exited before becoming ready (code ${instance.exit_code})`);
  }
  await stop(instance);
  throw new Error(`VS Code Server did not become ready within ${startTimeoutMs}ms`);
}

function get(id: string) {
  const instance = instances.get(id);
  if (!instance) throw new Error(`Unknown VS Code workspace: ${id}`);
  return instance;
}

function livePorts() {
  return [...instances.values()]
    .filter((instance) => !processExited(instance.process))
    .map((instance) => instance.port);
}

async function start(input: { id?: string; name?: string; workspace: string }) {
  const workspace = validateWorkspacePath(input.workspace);
  if (!(await stat(workspace).catch(() => null))?.isDirectory()) {
    throw new Error('workspace must be an existing directory');
  }
  const id = validateId(input.id ?? instanceIdFor(workspace));
  const existing = instances.get(id);
  if (existing && existing.status === 'running' && existing.workspace === workspace) {
    return publicInstance(existing);
  }
  if (existing) await stop(existing);
  await ensureCli();

  const port = await findFreePort({
    min: portMin,
    max: portMax,
    host: bindHost,
    taken: livePorts(),
  });
  const root = join(dataDir, id);
  const serverData = join(root, 'server-data');
  const cliData = join(root, 'cli-data');
  await Promise.all([mkdir(serverData, { recursive: true }), mkdir(cliData, { recursive: true })]);

  const child = spawn(binary, serverArgs({ bindHost, port, serverData, cliData, workspace }), {
    stdio: ['ignore', 'pipe', 'pipe'],
    detached: true,
  });
  const instance: Instance = {
    id,
    name: input.name?.trim() || 'VS Code',
    workspace,
    host: bindHost,
    port,
    url: `http://${bindHost}:${port}/`,
    pid: child.pid ?? null,
    started_at: new Date().toISOString(),
    status: 'starting',
    process: child,
  };
  instances.set(id, instance);
  child.once('exit', (code) => {
    instance.exit_code = code;
    instance.status = code === 0 ? 'stopped' : 'failed';
  });
  child.stderr?.on('data', (chunk) => process.stderr.write(`[vscode:${id}] ${chunk}`));

  await waitUntilReady(instance);
  return publicInstance(instance);
}

iii.registerFunction('vscode::start', start, {
  description: 'Start (or reuse) a VS Code Workbench for an absolute workspace directory.',
  request_format: schema(
    { id: { type: 'string' }, name: { type: 'string' }, workspace: { type: 'string' } },
    ['workspace'],
  ),
  response_format: schema(instanceFields, ['id', 'workspace', 'host', 'port', 'status']),
});

iii.registerFunction(
  'vscode::instances::list',
  async () => ({ instances: [...instances.values()].map(publicInstance) }),
  {
    description: 'List the VS Code Workbench processes this worker owns.',
    request_format: schema({}),
    response_format: schema({ instances: { type: 'array', items: schema(instanceFields) } }, [
      'instances',
    ]),
  },
);

iii.registerFunction(
  'vscode::stop',
  async (input: { id: string }) => {
    const instance = get(input.id);
    await stop(instance);
    return publicInstance(instance);
  },
  {
    description: 'Stop a VS Code Workbench process group.',
    request_format: schema({ id: { type: 'string' } }, ['id']),
    response_format: schema(instanceFields, ['id', 'status']),
  },
);

iii.registerFunction(
  'vscode::delete',
  async (input: { id: string; delete_profile?: boolean }) => {
    const instance = get(input.id);
    await stop(instance);
    instances.delete(input.id);
    if (input.delete_profile) await rm(join(dataDir, input.id), { recursive: true, force: true });
    return { deleted: true };
  },
  {
    description: 'Stop a VS Code Workbench process and optionally remove its isolated profile.',
    request_format: schema({ id: { type: 'string' }, delete_profile: { type: 'boolean' } }, ['id']),
    response_format: schema({ deleted: { type: 'boolean' } }, ['deleted']),
  },
);

type UiAsset = {
  file: string;
  type: 'console:script' | 'console:style';
  content_type: string;
  content: string;
};

const uiAssets: Record<string, UiAsset> = {
  'vscode/page.js': {
    file: 'page.js',
    type: 'console:script',
    content_type: 'text/javascript',
    content: uiPage,
  },
  'vscode/styles.css': {
    file: 'styles.css',
    type: 'console:style',
    content_type: 'text/css',
    content: uiStyles,
  },
};
const uiWatch = process.env.III_VSCODE_UI_WATCH;
const uiWatchEnabled = Boolean(uiWatch);
const uiWatchDir =
  uiWatchEnabled && uiWatch !== '1' && uiWatch !== 'true'
    ? (uiWatch as string)
    : join(dirname(fileURLToPath(import.meta.url)), '..', '..', 'ui', 'dist');

async function uiContent(path: string) {
  const asset = uiAssets[path];
  if (!asset) throw new Error(`unknown ui asset: ${path}`);
  const content = uiWatchEnabled
    ? await readFile(join(uiWatchDir, asset.file), 'utf8')
    : asset.content;
  return { content, content_type: asset.content_type };
}

iii.registerFunction('vscode::ui-content', (input: { path: string }) => uiContent(input.path), {
  description: 'Serve the injectable VS Code Console page assets.',
  metadata: { internal: true },
  request_format: schema({ path: { type: 'string' } }, ['path']),
  response_format: schema({ content: { type: 'string' }, content_type: { type: 'string' } }, [
    'content',
    'content_type',
  ]),
});

function registerUiAsset(path: string) {
  return iii.registerTrigger({
    type: uiAssets[path].type,
    function_id: 'vscode::ui-content',
    config: { path },
  });
}

const uiTriggers = new Map(Object.keys(uiAssets).map((path) => [path, registerUiAsset(path)]));

if (uiWatchEnabled) {
  const pending = new Map<string, NodeJS.Timeout>();
  watch(uiWatchDir, (_event, file) => {
    const path = Object.keys(uiAssets).find((key) => uiAssets[key].file === file);
    if (!path) return;
    clearTimeout(pending.get(path));
    pending.set(
      path,
      setTimeout(() => {
        const previous = uiTriggers.get(path);
        uiTriggers.set(path, registerUiAsset(path));
        previous?.unregister();
        console.error(`[vscode] reloaded ui asset ${path}`);
      }, 150),
    );
  });
  console.error(`[vscode] serving ui assets from ${uiWatchDir}`);
}

async function shutdown() {
  await Promise.all([...instances.values()].map(stop));
  await iii.shutdown();
}

process.on('SIGTERM', () => void shutdown());
process.on('SIGINT', () => void shutdown());
