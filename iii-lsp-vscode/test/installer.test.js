const fs = require('node:fs/promises');
const os = require('node:os');
const path = require('node:path');
const test = require('node:test');
const assert = require('node:assert/strict');

const installer = require('../installer');
const {
  SERVER_VERSION,
  ensureServerBinary,
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
          assert.equal(section, 'iii-lsp');
          return configuration;
        },
      },
    },
    updates,
  };
}

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
    assert.equal(installer.getPlatformTarget(platform, arch), expected);
  }
});

test('unsupported platform error', () => {
  assert.throws(
    () => installer.getPlatformTarget('freebsd', 'x64'),
    /Unsupported iii-lsp platform: freebsd\/x64/
  );
});

test('archive extension by platform', () => {
  assert.equal(
    installer.getArchiveName('aarch64-apple-darwin', 'darwin'),
    'iii-lsp-aarch64-apple-darwin.tar.gz'
  );
  assert.equal(
    installer.getArchiveName('x86_64-unknown-linux-gnu', 'linux'),
    'iii-lsp-x86_64-unknown-linux-gnu.tar.gz'
  );
  assert.equal(
    installer.getArchiveName('x86_64-pc-windows-msvc', 'win32'),
    'iii-lsp-x86_64-pc-windows-msvc.zip'
  );
});

test('binary filename by platform', () => {
  assert.equal(installer.getBinaryName('win32'), 'iii-lsp.exe');
  assert.equal(installer.getBinaryName('linux'), 'iii-lsp');
});

test('pinned release url', () => {
  assert.equal(installer.SERVER_VERSION, '0.1.0');
  assert.equal(installer.RELEASE_TAG, 'iii-lsp/v0.1.0');
  assert.equal(
    installer.RELEASE_BASE_URL,
    'https://github.com/iii-hq/workers/releases/download/iii-lsp/v0.1.0'
  );
});

test('checksum and download url', () => {
  assert.equal(
    installer.getChecksumName('aarch64-apple-darwin'),
    'iii-lsp-aarch64-apple-darwin.sha256'
  );
  assert.equal(
    installer.getDownloadUrl('iii-lsp-aarch64-apple-darwin.tar.gz'),
    'https://github.com/iii-hq/workers/releases/download/iii-lsp/v0.1.0/iii-lsp-aarch64-apple-darwin.tar.gz'
  );
});

test('parses sha256 files from GitHub release assets', () => {
  assert.equal(
    normalizeChecksum(
      '04dc683db6f30a983017e71ed7f4aa3ccb7fc5124261274d3a733a8e77c66da4  iii-lsp-aarch64-apple-darwin.tar.gz\n'
    ),
    '04dc683db6f30a983017e71ed7f4aa3ccb7fc5124261274d3a733a8e77c66da4'
  );
});

test('rejects malformed sha256 files', () => {
  assert.throws(() => normalizeChecksum('not-a-checksum'), /Invalid sha256/);
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
        assert.equal(
          url,
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

  assert.equal(binaryPath, path.join(tempDir, 'server', SERVER_VERSION, 'aarch64-apple-darwin', 'iii-lsp'));
  assert.equal(
    downloaded[0][0],
    getDownloadUrl('iii-lsp-aarch64-apple-darwin.tar.gz')
  );
  assert.deepEqual(fakeVscode.updates, [['serverPath', binaryPath, 'global']]);
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

  assert.equal(binaryPath, configuredPath);
  assert.deepEqual(calls, [configuredPath]);
  assert.deepEqual(fakeVscode.updates, []);
});

test('rejects archives whose checksum does not match', async () => {
  const tempDir = await fs.mkdtemp(path.join(os.tmpdir(), 'iii-lsp-vscode-test-'));
  const fakeVscode = createFakeVscode();
  const expectedChecksum = 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa';
  const actualChecksum = 'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb';

  await assert.rejects(
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
    ),
    /Checksum mismatch/
  );
});
