'use strict';

const SERVER_VERSION = '0.1.0';
const RELEASE_TAG = `iii-lsp/v${SERVER_VERSION}`;
const RELEASE_BASE_URL = `https://github.com/iii-hq/workers/releases/download/${RELEASE_TAG}`;

const PLATFORM_TARGETS = {
  'darwin/arm64': 'aarch64-apple-darwin',
  'darwin/x64': 'x86_64-apple-darwin',
  'linux/arm64': 'aarch64-unknown-linux-gnu',
  'linux/arm': 'armv7-unknown-linux-gnueabihf',
  'linux/x64': 'x86_64-unknown-linux-gnu',
  'win32/arm64': 'aarch64-pc-windows-msvc',
  'win32/ia32': 'i686-pc-windows-msvc',
  'win32/x64': 'x86_64-pc-windows-msvc',
};

function getPlatformTarget(platform = process.platform, arch = process.arch) {
  const key = `${platform}/${arch}`;
  const target = PLATFORM_TARGETS[key];

  if (!target) {
    throw new Error(`Unsupported iii-lsp platform: ${key}`);
  }

  return target;
}

function getArchiveName(target, platform = process.platform) {
  return `iii-lsp-${target}${platform === 'win32' ? '.zip' : '.tar.gz'}`;
}

function getBinaryName(platform = process.platform) {
  return platform === 'win32' ? 'iii-lsp.exe' : 'iii-lsp';
}

function getChecksumName(target) {
  return `iii-lsp-${target}.sha256`;
}

function getDownloadUrl(assetName) {
  return `${RELEASE_BASE_URL}/${assetName}`;
}

module.exports = {
  SERVER_VERSION,
  RELEASE_TAG,
  RELEASE_BASE_URL,
  getArchiveName,
  getBinaryName,
  getChecksumName,
  getDownloadUrl,
  getPlatformTarget,
};
