import { createHmac, randomBytes, timingSafeEqual } from 'node:crypto';
import type { IIIClient } from 'iii-sdk';
import { z } from 'zod';
import { exec } from './host.js';

export const Reply = z.object({ id: z.guid(), text: z.string().min(1) });
const Snapshot = z.strictObject({ cursor: z.string(), replies: z.array(Reply.strict()) });
const MAX_RESULT_BYTES = 48 * 1024;
const TAG_BYTES = 32;
const MAX_STDOUT_BYTES = 4 * Math.ceil((MAX_RESULT_BYTES + TAG_BYTES) / 3);
// The IDE's pinned Rust SDK redacts env values with `secret` in the key,
// but records stdout verbatim. Only the one-use pad goes in that field.
const SECRET_PAD_ENV = 'CLAUDE_REPLY_SECRET_PAD';
type ReadMode = 'last' | 'baseline' | 'since';

// Runs on the *terminal* host. Paths and conversation records never leave it.
// -I -S excludes user configuration, sitecustomize and PYTHONPATH from startup.
const READER = String.raw`
import base64, hashlib, hmac, json, os, re, stat, sys
from contextlib import ExitStack

UUID = re.compile(r'[0-9a-fA-F]{8}(?:-[0-9a-fA-F]{4}){3}-[0-9a-fA-F]{12}')
DIR = os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW
FILE = os.O_RDONLY | os.O_NONBLOCK | os.O_NOFOLLOW
MAX_FILE, MAX_LINE, MAX_RESULT = 8 << 20, 1 << 20, 48 << 10

# Standalone annotations do not move the conversation leaf. Unknown formats,
# content replacements and layouts that require relinking are unsupported.
ANNOTATIONS = {'summary', 'metadata', 'file-history-snapshot', 'queue-operation',
               'custom-title', 'ai-title', 'agent-name', 'agent-color', 'agent-setting',
               'last-prompt', 'mode', 'permission-mode', 'tag', 'pr-link', 'attribution-snapshot',
               'todo', 'worktree-state', 'tools-used', 'atis-latch', 'derived-from', 'relocated'}
HIDDEN = ('isMeta', 'isCompactSummary', 'isVirtual', 'isVisibleInTranscriptOnly', 'isApiErrorMessage')

def require(ok):
    if not ok:
        raise ValueError()

def reject_constant(_):
    raise ValueError()

def unique_members(pairs):
    obj = dict(pairs)
    require(len(obj) == len(pairs))
    return obj

def transcript(sid, stack):
    base = os.environ.get('CLAUDE_CONFIG_DIR') or os.path.expanduser('~/.claude')
    path = os.path.realpath(os.path.join(base, 'projects'))
    # The configured root itself can be a symlink to a shared store. Pin its
    # resolved components; never follow a symlink below it (including the file).
    root = os.open('/', DIR)
    try:
        for component in path.split('/'):
            if component:
                child = os.open(component, DIR, dir_fd=root)
                os.close(root)
                root = child
        found = None
        with os.scandir(root) as projects:
            for count, project in enumerate(projects, 1):
                require(count <= 4096 and not project.is_symlink())
                if not project.is_dir(follow_symlinks=False):
                    continue
                sub = os.open(project.name, DIR, dir_fd=root)
                try:
                    try:
                        info = os.stat(sid + '.jsonl', dir_fd=sub, follow_symlinks=False)
                    except FileNotFoundError:
                        continue
                    require(found is None and stat.S_ISREG(info.st_mode))
                    fd = os.open(sid + '.jsonl', FILE, dir_fd=sub)
                    found = stack.enter_context(os.fdopen(fd, 'rb'))
                    actual = os.fstat(fd)
                    require(os.path.samestat(actual, info) and actual.st_nlink == 1)
                finally:
                    os.close(sub)
        require(found is not None)
        return found
    finally:
        os.close(root)

# These content blocks can also be nested inside a tool result.
USER_CONTENT = ('text', 'image', 'search_result', 'document')
RESULT_CONTENT = (*USER_CONTENT, 'tool_reference')

def validate_content_block(part, allowed):
    require(isinstance(part, dict) and part.get('type') in allowed)
    kind = part['type']
    if kind == 'text':
        require(isinstance(part.get('text'), str))
    elif kind == 'tool_reference':
        require(isinstance(part.get('tool_name'), str))
    elif kind == 'search_result':
        require(isinstance(part.get('source'), str) and isinstance(part.get('title'), str))
        require(isinstance(part.get('content'), list))
        for text in part['content']:
            validate_content_block(text, ('text',))
    else:  # image or document
        source = part.get('source')
        require(isinstance(source, dict))
        source_type = source.get('type')
        if source_type == 'url':
            require(isinstance(source.get('url'), str))
        elif source_type == 'base64':
            media_types = (('image/jpeg', 'image/png', 'image/gif', 'image/webp')
                           if kind == 'image' else ('application/pdf',))
            require(source.get('media_type') in media_types and isinstance(source.get('data'), str))
        elif kind == 'document' and source_type == 'text':
            require(source.get('media_type') == 'text/plain' and isinstance(source.get('data'), str))
        elif kind == 'document' and source_type == 'content':
            content = source.get('content')
            require(isinstance(content, (str, list)))
            if isinstance(content, list):
                for block in content:
                    validate_content_block(block, ('text', 'image'))
        else:
            require(False)

def message_content(row, role):
    message = row['message']
    require(isinstance(message, dict) and message.get('role', role) == role)
    content = message['content']
    if isinstance(content, str):
        return content
    require(isinstance(content, list))
    for part in content:
        require(isinstance(part, dict) and isinstance(part.get('type'), str))
        kind = part['type']
        if kind == 'text' or (role == 'user' and kind in USER_CONTENT):
            validate_content_block(part, USER_CONTENT)
        elif role == 'assistant' and kind == 'thinking':
            require(isinstance(part.get('thinking'), str) and isinstance(part.get('signature'), str))
        elif role == 'assistant' and kind == 'redacted_thinking':
            require(isinstance(part.get('data'), str))
        elif role == 'assistant' and kind == 'tool_use':
            require(isinstance(part.get('id'), str) and part['id'])
            require(isinstance(part.get('name'), str) and part['name'] and 'input' in part)
        elif role == 'user' and kind == 'tool_result':
            require(isinstance(part.get('tool_use_id'), str))
            require('is_error' not in part or isinstance(part['is_error'], bool))
            if 'content' in part:
                result = part['content']
                require(isinstance(result, (str, list)))
                if isinstance(result, list):
                    for block in result:
                        validate_content_block(block, RESULT_CONTENT)
        else:
            raise ValueError()
    return content

def visible(content):
    if isinstance(content, str):
        return content
    return '\n'.join(part['text'] for part in content if part['type'] == 'text' and part['text'].strip())

def scan(sid, mode, previous):
    require(UUID.fullmatch(sid) and mode in ('last', 'baseline', 'since'))
    with ExitStack() as stack:
        f = transcript(sid, stack)
        info = os.fstat(f.fileno())
        require(info.st_size <= MAX_FILE)
        data = f.read(MAX_FILE + 1)
        after = os.fstat(f.fileno())
        require(len(data) == info.st_size == after.st_size and info.st_mtime_ns == after.st_mtime_ns)
    end = data.rfind(b'\n') + 1
    require(len(data) - end <= MAX_LINE)
    identity = str(info.st_dev) + ':' + str(info.st_ino)
    def cursor(offset):
        digest = hashlib.sha256(memoryview(data)[:offset]).hexdigest()
        return identity + ':' + str(offset) + ':' + digest
    boundary = end
    if mode == 'since':
        offset = int(previous.split(':')[2])
        if 0 <= offset <= end and cursor(offset) == previous:
            boundary = offset
    nodes, leaf, pos = {}, None, 0
    for count in range(65537):
        if pos == end:
            break
        stop = data.index(b'\n', pos) + 1
        require(count < 65536 and stop - pos <= MAX_LINE)
        row = json.loads(data[pos:stop].decode('utf-8'), parse_constant=reject_constant,
                         object_pairs_hook=unique_members)
        pos = stop
        require(isinstance(row, dict) and isinstance(row.get('type'), str))
        kind = row['type']
        if kind in ANNOTATIONS:
            continue
        require(kind in ('assistant', 'user', 'system', 'progress', 'attachment'))
        require(row.get('subtype') != 'compact_boundary')
        key = row.get('uuid')
        require(isinstance(key, str) and UUID.fullmatch(key) and key not in nodes)
        side = row.get('isSidechain', False)
        require(isinstance(side, bool))
        nodes[key] = (row, pos)
        if (kind in ('user', 'assistant') and not side and not row.get('teamName')
                and not any(row.get(flag) for flag in HIDDEN)):
            leaf = key
    replies = []
    while leaf is not None:
        row, pos = nodes.pop(leaf)  # missing parents, duplicates and cycles fail closed
        require(not row.get('isSidechain') and not row.get('teamName') and row.get('sessionId', sid) == sid)
        if row['type'] in ('assistant', 'user'):
            content = message_content(row, row['type'])
        parent = row['parentUuid']
        if parent is not None:
            require(parent in nodes and nodes[parent][1] < pos)
        if row['type'] == 'assistant' and not any(row.get(k) for k in HIDDEN):
            text = visible(content)
            if text.strip() and (mode == 'last' or pos > boundary):
                if mode != 'last' or not replies:
                    replies.append({'id': leaf, 'text': text})
        leaf = parent
    out = json.dumps({'cursor': cursor(end), 'replies': replies[::-1]}, separators=(',', ':'))
    require(len(out) <= MAX_RESULT)
    return out

try:
    pad = base64.b64decode(os.environ.pop('${SECRET_PAD_ENV}'), validate=True)
    require(len(pad) == MAX_RESULT + 32)
    plain = scan(*sys.argv[1:]).encode('ascii')
    masked = bytes(a ^ b for a, b in zip(plain, pad[32:]))
    tag = hmac.new(pad[:32], masked, hashlib.sha256).digest()
    sys.stdout.write(base64.b64encode(tag + masked).decode('ascii'))
except Exception:
    sys.exit(1)
`;

export async function readTranscript(
  iii: IIIClient,
  env: Record<string, string>,
  id: string,
  mode: ReadMode,
  cursor = '',
) {
  try {
    // A fresh random pad for each read keeps the recorded stdout opaque even
    // when IDE payload tracing is on. Authenticate before decoding the JSON.
    const pad = randomBytes(MAX_RESULT_BYTES + TAG_BYTES);
    const r = await exec(iii, 'python3', {
      args: ['-I', '-S', '-B', '-c', READER, id, mode, cursor],
      env: { ...env, [SECRET_PAD_ENV]: pad.toString('base64') },
      timeoutMs: 5_000,
    });
    if (r.exit_code === 0 && r.stdout.length <= MAX_STDOUT_BYTES) {
      const packet = Buffer.from(r.stdout, 'base64');
      if (packet.length <= TAG_BYTES || packet.length > pad.length) {
        throw new Error('invalid host packet');
      }
      const masked = packet.subarray(TAG_BYTES);
      const tag = createHmac('sha256', pad.subarray(0, TAG_BYTES)).update(masked).digest();
      if (timingSafeEqual(packet.subarray(0, TAG_BYTES), tag)) {
        const plain = masked.map((b, i) => b ^ pad[i + TAG_BYTES]);
        return Snapshot.parse(JSON.parse(new TextDecoder('utf-8', { fatal: true }).decode(plain)));
      }
    }
  } catch {
    // Never propagate host diagnostics, paths or raw conversation contents.
  }
  throw new Error('transcript unavailable');
}
