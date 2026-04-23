import fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { createRequire } from 'node:module';
import * as tar from 'tar';
import { describe, expect, test } from 'vitest';

const require = createRequire(import.meta.url);
const installer = require('../installer');
const {
  SERVER_VERSION,
  ensureServerBinary,
  extractArchive,
  fileExists,
  getDownloadUrl,
  normalizeChecksum,
} = installer;

function createFakeVscode(initialServerPath = '') {
  const updates = [];
  const configuration = {
    get(key) {
      if (key === 'serverPath') {
        return initialServerPath;
      }

      return undefined;
    },
    update(key, value, target) {
      updates.push([key, value, target]);
      return Promise.resolve();
    },
  };

  return {
    api: {
      ConfigurationTarget: {
        Global: 'global',
      },
      workspace: {
        getConfiguration(section) {
          expect(section).toBe('iii-lsp');
          return configuration;
        },
      },
    },
    updates,
  };
}

describe('iii-lsp-vscode installer', () => {
  test('supported platform mapping', () => {
    const cases = [
      ['darwin', 'arm64', 'aarch64-apple-darwin'],
      ['darwin', 'x64', 'x86_64-apple-darwin'],
      ['linux', 'arm64', 'aarch64-unknown-linux-gnu'],
      ['linux', 'arm', 'armv7-unknown-linux-gnueabihf'],
      ['linux', 'x64', 'x86_64-unknown-linux-gnu'],
      ['win32', 'arm64', 'aarch64-pc-windows-msvc'],
      ['win32', 'ia32', 'i686-pc-windows-msvc'],
      ['win32', 'x64', 'x86_64-pc-windows-msvc'],
    ];

    for (const [platform, arch, expected] of cases) {
      expect(installer.getPlatformTarget(platform, arch)).toBe(expected);
    }
  });

  test('unsupported platform error', () => {
    expect(() => installer.getPlatformTarget('freebsd', 'x64')).toThrow(
      /Unsupported iii-lsp platform: freebsd\/x64/
    );
  });

  test('archive extension by platform', () => {
    expect(installer.getArchiveName('aarch64-apple-darwin', 'darwin')).toBe(
      'iii-lsp-aarch64-apple-darwin.tar.gz'
    );
    expect(installer.getArchiveName('x86_64-unknown-linux-gnu', 'linux')).toBe(
      'iii-lsp-x86_64-unknown-linux-gnu.tar.gz'
    );
    expect(installer.getArchiveName('x86_64-pc-windows-msvc', 'win32')).toBe(
      'iii-lsp-x86_64-pc-windows-msvc.zip'
    );
  });

  test('binary filename by platform', () => {
    expect(installer.getBinaryName('win32')).toBe('iii-lsp.exe');
    expect(installer.getBinaryName('linux')).toBe('iii-lsp');
  });

  test('pinned release url', () => {
    expect(installer.SERVER_VERSION).toBe('0.1.0');
    expect(installer.RELEASE_TAG).toBe('iii-lsp/v0.1.0');
    expect(installer.RELEASE_BASE_URL).toBe(
      'https://github.com/iii-hq/workers/releases/download/iii-lsp/v0.1.0'
    );
  });

  test('checksum and download url', () => {
    expect(installer.getChecksumName('aarch64-apple-darwin')).toBe(
      'iii-lsp-aarch64-apple-darwin.sha256'
    );
    expect(installer.getDownloadUrl('iii-lsp-aarch64-apple-darwin.tar.gz')).toBe(
      'https://github.com/iii-hq/workers/releases/download/iii-lsp/v0.1.0/iii-lsp-aarch64-apple-darwin.tar.gz'
    );
  });

  test('parses sha256 files from GitHub release assets', () => {
    expect(
      normalizeChecksum(
        '04dc683db6f30a983017e71ed7f4aa3ccb7fc5124261274d3a733a8e77c66da4  iii-lsp-aarch64-apple-darwin.tar.gz\n'
      )
    ).toBe('04dc683db6f30a983017e71ed7f4aa3ccb7fc5124261274d3a733a8e77c66da4');
  });

  test('rejects malformed sha256 files', () => {
    expect(() => normalizeChecksum('not-a-checksum')).toThrow(/Invalid sha256/);
  });

  test('fileExists requires a regular executable binary on non-win32', async () => {
    const tempDir = await fs.mkdtemp(path.join(os.tmpdir(), 'iii-lsp-vscode-test-'));
    const tempFile = path.join(tempDir, 'iii-lsp');

    await fs.mkdir(path.join(tempDir, 'subdir'), { recursive: true });
    await fs.writeFile(tempFile, 'fake binary');

    expect(await fileExists(path.join(tempDir, 'subdir'))).toBe(false);
    expect(await fileExists(tempFile)).toBe(false);

    if (process.platform !== 'win32') {
      await fs.chmod(tempFile, 0o755);
    }

    expect(await fileExists(tempFile)).toBe(true);
  });

  test('extractArchive unpacks a tar.gz binary and marks it executable', async () => {
    if (process.platform === 'win32') {
      return;
    }

    const tempDir = await fs.mkdtemp(path.join(os.tmpdir(), 'iii-lsp-vscode-test-'));
    const archiveDir = path.join(tempDir, 'archive');
    const installDir = path.join(tempDir, 'install');
    const archivePath = path.join(tempDir, 'iii-lsp-linux.tar.gz');
    const binaryPath = path.join(archiveDir, 'iii-lsp');

    await fs.mkdir(archiveDir, { recursive: true });
    await fs.writeFile(binaryPath, '#!/bin/sh\necho iii-lsp\n');
    await fs.chmod(binaryPath, 0o644);
    await tar.c(
      {
        gzip: true,
        cwd: archiveDir,
        file: archivePath,
      },
      ['iii-lsp']
    );

    const extractedPath = await extractArchive(archivePath, installDir, 'linux');
    const extractedStat = await fs.stat(extractedPath);

    expect(extractedPath).toBe(path.join(installDir, 'iii-lsp'));
    expect(await fileExists(extractedPath)).toBe(true);
    expect((extractedStat.mode & 0o111) !== 0).toBe(true);
  });

  test('installs missing binary and saves global serverPath', async () => {
    const tempDir = await fs.mkdtemp(path.join(os.tmpdir(), 'iii-lsp-vscode-test-'));
    const fakeVscode = createFakeVscode();
    const downloaded = [];
    const checksum = 'e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855';

    const binaryPath = await ensureServerBinary(
      { globalStorageUri: { fsPath: tempDir } },
      fakeVscode.api,
      {
        platform: 'darwin',
        arch: 'arm64',
        fileExists: async () => false,
        downloadText: async (url) => {
          expect(url).toBe(
            'https://github.com/iii-hq/workers/releases/download/iii-lsp/v0.1.0/iii-lsp-aarch64-apple-darwin.sha256'
          );
          return `${checksum}  iii-lsp-aarch64-apple-darwin.tar.gz\n`;
        },
        downloadFile: async (url, destination) => {
          downloaded.push([url, destination]);
          await fs.writeFile(destination, '');
        },
        sha256File: async () => checksum,
        extractArchive: async (_archivePath, installDir) => {
          const installedPath = path.join(installDir, 'iii-lsp');
          await fs.mkdir(installDir, { recursive: true });
          await fs.writeFile(installedPath, 'fake binary');
          return installedPath;
        },
      }
    );

    expect(binaryPath).toBe(
      path.join(tempDir, 'server', SERVER_VERSION, 'aarch64-apple-darwin', 'iii-lsp')
    );
    expect(downloaded[0][0]).toBe(getDownloadUrl('iii-lsp-aarch64-apple-darwin.tar.gz'));
    expect(fakeVscode.updates).toEqual([['serverPath', binaryPath, 'global']]);
  });

  test('uses valid configured serverPath without downloading', async () => {
    const tempDir = await fs.mkdtemp(path.join(os.tmpdir(), 'iii-lsp-vscode-test-'));
    const configuredPath = path.join(tempDir, 'configured-server');
    await fs.writeFile(configuredPath, 'fake binary');
    const fakeVscode = createFakeVscode(configuredPath);
    const calls = [];

    const binaryPath = await ensureServerBinary(
      { globalStorageUri: { fsPath: tempDir } },
      fakeVscode.api,
      {
        platform: 'darwin',
        arch: 'arm64',
        fileExists: async (filePath) => {
          calls.push(filePath);
          return filePath === configuredPath;
        },
        downloadText: async () => {
          throw new Error('downloadText should not be called');
        },
        downloadFile: async () => {
          throw new Error('downloadFile should not be called');
        },
        sha256File: async () => {
          throw new Error('sha256File should not be called');
        },
        extractArchive: async () => {
          throw new Error('extractArchive should not be called');
        },
      }
    );

    expect(binaryPath).toBe(configuredPath);
    expect(calls).toEqual([configuredPath]);
    expect(fakeVscode.updates).toEqual([]);
  });

  test('rejects archives whose checksum does not match', async () => {
    const tempDir = await fs.mkdtemp(path.join(os.tmpdir(), 'iii-lsp-vscode-test-'));
    const fakeVscode = createFakeVscode();
    const expectedChecksum = 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa';
    const actualChecksum = 'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb';

    await expect(
      ensureServerBinary(
        { globalStorageUri: { fsPath: tempDir } },
        fakeVscode.api,
        {
          platform: 'linux',
          arch: 'x64',
          fileExists: async () => false,
          downloadText: async () => `${expectedChecksum}  iii-lsp-x86_64-unknown-linux-gnu.tar.gz\n`,
          downloadFile: async (_url, destination) => {
            await fs.writeFile(destination, '');
          },
          sha256File: async () => actualChecksum,
          extractArchive: async () => {
            throw new Error('extractArchive should not be called');
          },
        }
      )
    ).rejects.toThrow(/Checksum mismatch/);
  });
});
