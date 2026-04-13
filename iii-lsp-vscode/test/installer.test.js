const test = require('node:test');
const assert = require('node:assert/strict');

const installer = require('../installer');

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
    installer.getArchiveName('win32', 'x64'),
    'iii-lsp-x86_64-pc-windows-msvc.zip'
  );
  assert.equal(
    installer.getArchiveName('linux', 'x64'),
    'iii-lsp-x86_64-unknown-linux-gnu.tar.gz'
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
