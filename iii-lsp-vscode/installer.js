'use strict';

const crypto = require('node:crypto');
const fs = require('node:fs');
const fsp = require('node:fs/promises');
const https = require('node:https');
const os = require('node:os');
const path = require('node:path');
const { pipeline } = require('node:stream/promises');

const extractZip = require('extract-zip');
const tar = require('tar');

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

function normalizeChecksum(text) {
  const checksum = String(text).trim().split(/\s+/, 1)[0].toLowerCase();

  if (!/^[0-9a-f]{64}$/.test(checksum)) {
    throw new Error('Invalid sha256 checksum file');
  }

  return checksum;
}

async function fileExists(filePath) {
  try {
    await fsp.access(filePath, fs.constants.F_OK);
    return true;
  } catch {
    return false;
  }
}

function fetchResponse(url, httpsModule, redirectsRemaining) {
  return new Promise((resolve, reject) => {
    const request = httpsModule.get(url, (response) => {
      const { statusCode, headers } = response;

      if (statusCode >= 300 && statusCode < 400 && headers.location) {
        response.resume();

        if (redirectsRemaining <= 0) {
          reject(new Error(`Too many redirects while fetching ${url}`));
          return;
        }

        resolve(fetchResponse(new URL(headers.location, url).href, httpsModule, redirectsRemaining - 1));
        return;
      }

      if (statusCode !== 200) {
        response.resume();
        reject(new Error(`Unexpected status code ${statusCode} for ${url}`));
        return;
      }

      resolve(response);
    });

    request.on('error', reject);
  });
}

async function downloadText(url, httpsModule = https, redirectsRemaining = 5) {
  const response = await fetchResponse(url, httpsModule, redirectsRemaining);

  return await new Promise((resolve, reject) => {
    const chunks = [];
    response.setEncoding('utf8');
    response.on('data', (chunk) => {
      chunks.push(chunk);
    });
    response.on('end', () => {
      resolve(chunks.join(''));
    });
    response.on('error', reject);
  });
}

async function downloadFile(url, destination, httpsModule = https, redirectsRemaining = 5) {
  await fsp.mkdir(path.dirname(destination), { recursive: true });
  const response = await fetchResponse(url, httpsModule, redirectsRemaining);
  const output = fs.createWriteStream(destination, { mode: 0o600 });

  await pipeline(response, output);
}

async function sha256File(filePath) {
  const hash = crypto.createHash('sha256');
  const input = fs.createReadStream(filePath);

  for await (const chunk of input) {
    hash.update(chunk);
  }

  return hash.digest('hex');
}

async function extractArchive(archivePath, installDir, platform = process.platform) {
  await fsp.rm(installDir, { recursive: true, force: true });
  await fsp.mkdir(installDir, { recursive: true });

  if (platform === 'win32') {
    await extractZip(archivePath, { dir: installDir });
  } else {
    await tar.x({ file: archivePath, cwd: installDir });
  }

  const binaryPath = path.join(installDir, getBinaryName(platform));

  if (platform !== 'win32') {
    await fsp.chmod(binaryPath, 0o755);
  }

  return binaryPath;
}

function getInstallPaths(context, platform = process.platform, arch = process.arch) {
  const target = getPlatformTarget(platform, arch);
  const installDir = path.join(context.globalStorageUri.fsPath, 'server', SERVER_VERSION, target);
  const binaryPath = path.join(installDir, getBinaryName(platform));
  const archivePath = path.join(os.tmpdir(), getArchiveName(target, platform));

  return {
    target,
    installDir,
    binaryPath,
    archivePath,
  };
}

async function ensureServerBinary(context, vscodeApi, options = {}) {
  const platform = options.platform ?? process.platform;
  const arch = options.arch ?? process.arch;
  const fileExistsImpl = options.fileExists ?? fileExists;
  const downloadTextImpl = options.downloadText ?? downloadText;
  const downloadFileImpl = options.downloadFile ?? downloadFile;
  const sha256FileImpl = options.sha256File ?? sha256File;
  const extractArchiveImpl = options.extractArchive ?? extractArchive;
  const configuration = vscodeApi.workspace.getConfiguration('iii-lsp');
  const configuredServerPath = configuration.get('serverPath') ?? '';
  const installPaths = getInstallPaths(context, platform, arch);

  if (configuredServerPath && (await fileExistsImpl(configuredServerPath))) {
    return configuredServerPath;
  }

  if (!(await fileExistsImpl(installPaths.binaryPath))) {
    const checksumName = getChecksumName(installPaths.target);
    const archiveName = getArchiveName(installPaths.target, platform);
    const checksumUrl = getDownloadUrl(checksumName);
    const archiveUrl = getDownloadUrl(archiveName);
    const checksum = normalizeChecksum(await downloadTextImpl(checksumUrl));

    await downloadFileImpl(archiveUrl, installPaths.archivePath);

    const actualChecksum = await sha256FileImpl(installPaths.archivePath);

    if (actualChecksum !== checksum) {
      throw new Error(`Checksum mismatch for ${archiveName}: expected ${checksum}, got ${actualChecksum}`);
    }

    const extractedPath = await extractArchiveImpl(
      installPaths.archivePath,
      installPaths.installDir,
      platform
    );

    if (extractedPath !== installPaths.binaryPath) {
      throw new Error(`Expected extracted binary at ${installPaths.binaryPath}, got ${extractedPath}`);
    }
  }

  if (configuredServerPath !== installPaths.binaryPath) {
    await configuration.update('serverPath', installPaths.binaryPath, vscodeApi.ConfigurationTarget.Global);
  }

  return installPaths.binaryPath;
}

module.exports = {
  SERVER_VERSION,
  RELEASE_TAG,
  RELEASE_BASE_URL,
  downloadFile,
  downloadText,
  ensureServerBinary,
  extractArchive,
  fileExists,
  getArchiveName,
  getBinaryName,
  getChecksumName,
  getDownloadUrl,
  getInstallPaths,
  getPlatformTarget,
  normalizeChecksum,
  sha256File,
};
