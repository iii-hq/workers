import { dirname, resolve } from 'node:path';
import chokidar, { type FSWatcher } from 'chokidar';
import { logger } from '../../runtime/otel.js';
import { Permissions } from './permissions.js';

export class PermissionsHandle {
  private permissions = Permissions.empty();
  private watcher: FSWatcher | null = null;

  constructor(readonly path: string) {
    this.path = resolve(path);
  }

  current(): Permissions {
    return this.permissions;
  }

  async load(): Promise<void> {
    await this.reload(true);
  }

  startWatching(): void {
    if (this.watcher) return;

    const dir = dirname(this.path);
    let timer: ReturnType<typeof setTimeout> | undefined;
    const onPermissionsChange = (p: string) => {
      if (resolve(p) !== this.path) return;
      clearTimeout(timer);
      timer = setTimeout(() => void this.reload(false), 150);
    };

    try {
      this.watcher = chokidar
        .watch(dir, {
          depth: 0,
          ignoreInitial: true,
          awaitWriteFinish: { stabilityThreshold: 100, pollInterval: 50 },
        })
        .on('change', onPermissionsChange)
        .on('add', onPermissionsChange);
    } catch (err) {
      logger.warn('permissions watcher failed to start; reloads will require restart', {
        path: dir,
        err: String(err),
      });
    }
  }

  async stop(): Promise<void> {
    await this.watcher?.close().catch(() => {});
    this.watcher = null;
  }

  private async reload(initial: boolean): Promise<void> {
    try {
      this.permissions = await Permissions.loadFromPath(this.path);
      logger.info(`${initial ? 'loaded' : 'reloaded'} iii-permissions.yaml`, {
        path: this.path,
        rules: this.permissions.ruleCount(),
      });
    } catch (err) {
      const meta = { path: this.path, err: String(err) };
      if (initial) {
        logger.warn(
          'iii-permissions.yaml not loaded; starting with empty permissions (every call needs approval)',
          meta,
        );
        this.permissions = Permissions.empty();
      } else {
        logger.warn('failed to reload iii-permissions.yaml; keeping previous version', meta);
      }
    }
  }
}

export async function loadAndWatch(path: string): Promise<PermissionsHandle> {
  const handle = new PermissionsHandle(path);
  await handle.load();
  handle.startWatching();
  return handle;
}
