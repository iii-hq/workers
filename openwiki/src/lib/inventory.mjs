import fs from 'node:fs/promises';
import path from 'node:path';
import crypto from 'node:crypto';

export const DEFAULT_EXCLUDES = [
  '.git',
  'node_modules',
  'dist',
  'build',
  '.venv',
  '__pycache__',
  'target',
  '.next',
  '.turbo',
  'coverage',
  '.cache',
];
export const DEFAULT_MAX_FILE_BYTES = 200_000;
export const LANG_BY_EXT = {
  '.ts': 'typescript',
  '.tsx': 'typescript',
  '.js': 'javascript',
  '.jsx': 'javascript',
  '.mjs': 'javascript',
  '.cjs': 'javascript',
  '.py': 'python',
  '.rs': 'rust',
  '.go': 'go',
  '.java': 'java',
  '.kt': 'kotlin',
  '.swift': 'swift',
  '.c': 'c',
  '.h': 'c',
  '.cpp': 'cpp',
  '.cc': 'cpp',
  '.hpp': 'cpp',
  '.rb': 'ruby',
  '.php': 'php',
  '.md': 'markdown',
  '.mdx': 'markdown',
  '.yml': 'yaml',
  '.yaml': 'yaml',
  '.json': 'json',
  '.toml': 'toml',
  '.sh': 'shell',
  '.html': 'html',
  '.css': 'css',
  '.scss': 'scss',
  '.sql': 'sql',
  '.proto': 'protobuf',
};

const ROOT_META_PRIO2 = new Set([
  'package.json',
  'pyproject.toml',
  'Cargo.toml',
  'go.mod',
  'tsconfig.json',
  'pnpm-workspace.yaml',
]);
const DOC_BASENAMES = ['README', 'CHANGELOG', 'CONTRIBUTING', 'LICENSE'];

function stripExt(base) {
  const i = base.lastIndexOf('.');
  return i > 0 ? base.slice(0, i) : base;
}

function isDocFile(relPath, ext) {
  const base = path.posix.basename(relPath);
  const stem = stripExt(base).toUpperCase();
  if (DOC_BASENAMES.includes(stem)) return true;
  if (relPath.startsWith('docs/')) return true;
  if (ext === '.md' || ext === '.mdx') return true;
  return false;
}

function computePriority(relPath, _ext, language) {
  const base = path.posix.basename(relPath);
  const stem = stripExt(base).toUpperCase();
  const atRoot = !relPath.includes('/');

  if (atRoot && (base === 'README.md' || base === 'README.mdx')) return 3;
  if (atRoot && stem === 'CHANGELOG') return 3;
  if (relPath.startsWith('docs/')) return 3;

  if (atRoot && ROOT_META_PRIO2.has(base)) return 2;
  if (atRoot && (stem === 'LICENSE' || stem === 'CONTRIBUTING')) return 2;
  const entryStems = ['index', 'main'];
  if (entryStems.includes(stripExt(base))) {
    if (atRoot) return 2;
    if (relPath.startsWith('src/') && relPath.split('/').length === 2) return 2;
  }

  if (language && language !== 'text') {
    if (['json', 'yaml', 'toml'].includes(language)) return 0;
    return 1;
  }
  return 0;
}

function matchesSuffixGlob(name, globs) {
  if (!globs?.length) return false;
  for (const g of globs) {
    if (!g) continue;
    if (g.startsWith('*')) {
      if (name.endsWith(g.slice(1))) return true;
    } else if (name === g) return true;
    else if (name.endsWith(g)) return true;
  }
  return false;
}

export async function inventoryRepo(repoDir, opts = {}) {
  const maxBytes = opts.maxFileBytes ?? DEFAULT_MAX_FILE_BYTES;
  const excludeGlobs = opts.excludeGlobs || [];
  const out = [];

  async function walk(dir) {
    let entries;
    try {
      entries = await fs.readdir(dir, { withFileTypes: true });
    } catch {
      return;
    }
    for (const e of entries) {
      const full = path.join(dir, e.name);
      if (e.isDirectory()) {
        if (DEFAULT_EXCLUDES.includes(e.name)) continue;
        if (matchesSuffixGlob(e.name, excludeGlobs)) continue;
        await walk(full);
      } else if (e.isFile()) {
        if (matchesSuffixGlob(e.name, excludeGlobs)) continue;
        const rel = path.relative(repoDir, full).split(path.sep).join('/');
        let st;
        try {
          st = await fs.stat(full);
        } catch {
          continue;
        }
        const size = st.size;
        const ext = path.extname(e.name).toLowerCase();
        const language = LANG_BY_EXT[ext] || 'text';
        const isDoc = isDocFile(rel, ext);
        const priority = computePriority(rel, ext, language);

        let truncated = false;
        let buf;
        try {
          const fh = await fs.open(full, 'r');
          try {
            const readLen = Math.min(size, maxBytes);
            buf = Buffer.alloc(readLen);
            if (readLen > 0) await fh.read(buf, 0, readLen, 0);
            if (size > maxBytes) truncated = true;
          } finally {
            await fh.close();
          }
        } catch {
          buf = Buffer.alloc(0);
        }
        const sha = crypto.createHash('sha1').update(buf).digest('hex').slice(0, 12);

        out.push({ relPath: rel, size, ext, language, isDoc, priority, sha, truncated });
      }
    }
  }

  await walk(repoDir);
  out.sort((a, b) => b.priority - a.priority || (a.relPath < b.relPath ? -1 : a.relPath > b.relPath ? 1 : 0));
  return out;
}

export async function readSourceFile(repoDir, relPath, maxBytes = 200_000) {
  const full = path.join(repoDir, relPath);
  // Bounded read: request maxBytes + 1 through a handle instead of loading the
  // whole file, so a giant file never lands in memory. The extra byte is only
  // the truncation probe.
  const fh = await fs.open(full, 'r');
  let bytesRead = 0;
  let buf;
  try {
    buf = Buffer.alloc(maxBytes + 1);
    ({ bytesRead } = await fh.read(buf, 0, maxBytes + 1, 0));
  } finally {
    await fh.close();
  }
  const truncated = bytesRead > maxBytes;
  const content = truncated
    ? `${buf.subarray(0, maxBytes).toString('utf8')}\n...[truncated]`
    : buf.subarray(0, bytesRead).toString('utf8');
  return { path: relPath, content, truncated };
}
