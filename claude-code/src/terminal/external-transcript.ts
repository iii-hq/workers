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
type ReadMode = 'last' | 'baseline' | 'since' | 'recent';

// Runs on the *terminal* host. Paths and conversation records never leave it.
// -I -S excludes user configuration, sitecustomize and PYTHONPATH from startup.
const READER = String.raw`
import base64, hashlib, hmac, json, os, re, stat, sys
from contextlib import ExitStack

UUID = re.compile(r'[0-9a-fA-F]{8}(?:-[0-9a-fA-F]{4}){3}-[0-9a-fA-F]{12}')
DIR = os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW
FILE = os.O_RDONLY | os.O_NONBLOCK | os.O_NOFOLLOW
# A complete record may be as long as the parsed window itself.
MAX_FILE, MAX_RESULT = 8 << 20, 48 << 10

# Standalone annotations do not move the conversation leaf. Unknown formats,
# content replacements and layouts that require relinking are unsupported.
ANNOTATIONS = {'summary', 'metadata', 'file-history-snapshot', 'queue-operation',
               'custom-title', 'ai-title', 'agent-name', 'agent-color', 'agent-setting',
               'last-prompt', 'mode', 'permission-mode', 'tag', 'pr-link', 'attribution-snapshot',
               'todo', 'worktree-state', 'tools-used', 'atis-latch', 'derived-from', 'relocated'}
HIDDEN = ('isMeta', 'isCompactSummary', 'isVirtual', 'isVisibleInTranscriptOnly', 'isApiErrorMessage')

# The CLI writes this record when it compacts a conversation. It starts the
# chain the conversation continues on, so the active chain ends there; only
# a viewer with no reply since then looks before it.
def compact_boundary(row):
    return row['type'] == 'system' and row.get('subtype') == 'compact_boundary'

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
            require(isinstance(part.get('tool_use_id'), str) and part['tool_use_id'])
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

# Cursors carry a digest of only the final bytes before their observed size,
# so a poll reads what was appended instead of rehashing the whole file.
TAIL = 1 << 16
TRUNCATED = '\n[reply truncated]'
CHAIN = ('assistant', 'user', 'system', 'progress', 'attachment')

def tail_digest(fd, observed):
    lo = max(0, observed - TAIL)
    chunk = os.pread(fd, observed - lo, lo)
    require(len(chunk) == observed - lo)
    return hashlib.sha256(chunk)

def cursor_text(identity, observed, offset, digest):
    prefix = f'{identity}:{observed}:'
    if observed == offset:
        return prefix + (digest.hexdigest() if digest else '0' * 64)
    # Partial watch pages also carry the delivery offset. Use compact
    # base64 here to leave room for it without shrinking the reply budget.
    tag = base64.urlsafe_b64encode(digest.digest()).decode('ascii') if digest else '0' * 44
    return f'{prefix}{tag}:{offset}'

def find_record(fd, key, before, low, sid):
    # Searches the bytes before a parsed range backwards for the record whose
    # own uuid field is key, starting at offset low or later: one starting
    # further back is past the search budget. A uuid named only inside another
    # record (a parentUuid, a logicalParentUuid or message text) does not
    # count. The record must pass the checks of a chain record in the range.
    needle = b'"' + key.encode('ascii') + b'"'
    # The newline just before low shows that a line starts at low.
    edge = max(0, low - 1)
    # Parents are usually close: start with small reads and grow them.
    rest, pos, size = b'', before, 1 << 16
    while pos > edge:
        step = min(size, pos - edge)
        size = min(size * 2, 1 << 20)
        pos -= step
        chunk = os.pread(fd, step, pos)
        require(len(chunk) == step)
        buf = chunk + rest
        first = buf.find(b'\n')
        if pos > 0 and first < 0:
            rest = buf
            continue
        # Only lines whose start is known, and not before low, count here.
        cut = 0 if pos == 0 and low == 0 else first + 1
        body = buf[cut:]
        if needle in body:
            line_end = pos + len(buf)
            for line in reversed(body.split(b'\n')[:-1]):
                line_start = line_end - len(line) - 1
                if needle in line:
                    row = json.loads(line.decode('utf-8'), parse_constant=reject_constant,
                                     object_pairs_hook=unique_members)
                    if isinstance(row, dict) and row.get('uuid') == key:
                        require(row.get('type') in CHAIN and row.get('isSidechain', False) is False)
                        require(not row.get('teamName') and row.get('sessionId', sid) == sid)
                        return row, line_end, line_start
                line_end = line_start
        rest = buf[:cut]
    raise ValueError()

def scan(sid, mode, previous):
    require(UUID.fullmatch(sid) and mode in ('last', 'baseline', 'since', 'recent'))
    with ExitStack() as stack:
        f = transcript(sid, stack)
        return read(f.fileno(), sid, mode, previous)

def read(fd, sid, mode, previous):
    info = os.fstat(fd)
    total = info.st_size
    identity = str(info.st_dev) + ':' + str(info.st_ino)
    boundary, pending, known = None, False, None
    if mode in ('since', 'recent'):
        fields = previous.split(':')
        # A cursor may end with the uuid and end offset of the chain tip it
        # validated; an older cursor without them still reads.
        saved = fields[-2:] if len(fields) in (6, 7) else None
        base = fields[:-2] if saved else fields
        observed = int(base[2])
        offset = int(base[4]) if mode == 'since' and len(base) == 5 else observed
        # A pending page is valid only if the final bytes of the previously
        # observed complete prefix still exist unchanged, not just what was
        # delivered. Rewrites earlier in the file are not detected.
        if 0 <= offset <= observed <= total and cursor_text(
                identity, observed, offset, tail_digest(fd, observed)) == ':'.join(base):
            boundary, pending = offset, mode == 'since' and offset < observed
            if saved and UUID.fullmatch(saved[0]) and re.fullmatch(r'[0-9]+', saved[1]) and int(
                    saved[1]) <= observed:
                known = (saved[0], int(saved[1]))
    incremental = boundary is not None and total - boundary <= MAX_FILE
    if incremental:
        # Parse only what changed since the cursor.
        lead = start = boundary
    else:
        # Parse only the final MAX_FILE bytes, plus the byte before them to
        # find the first complete line.
        lead, start = max(0, total - MAX_FILE - 1), None
    data = os.pread(fd, total - lead + 1, lead)
    require(len(data) == total - lead)
    # File offsets are absolute; data starts at offset lead.
    # A trailing line without its newline is still being written: ignore it.
    end = lead + data.rfind(b'\n') + 1
    if start is None:
        start = lead + data.index(b'\n') + 1 if total > MAX_FILE else 0
        # A window without one complete record of its own is covered by a line
        # longer than the window: fail closed, never report an earlier or empty chain.
        require(start == 0 or start < end)
    if boundary is None:
        boundary = end
    # A historical search reads at most MAX_FILE bytes before the parsed start.
    low = max(0, start - MAX_FILE)
    def cursor(observed, offset, sizing=False):
        digest = None if sizing else tail_digest(fd, observed)
        return cursor_text(identity, observed, offset, digest) + ancestry
    nodes, leaf, i = {}, None, start - lead
    for count in range(65537):
        if i == end - lead:
            break
        stop = data.index(b'\n', i) + 1
        require(count < 65536)
        row = json.loads(data[i:stop].decode('utf-8'), parse_constant=reject_constant,
                         object_pairs_hook=unique_members)
        i = stop
        require(isinstance(row, dict) and isinstance(row.get('type'), str))
        kind = row['type']
        if kind in ANNOTATIONS:
            continue
        if kind not in CHAIN:
            # Newer CLI records without a uuid (session-owner) cannot be a
            # chain node. An unknown record that can be one is unsupported.
            require('uuid' not in row)
            continue
        key = row.get('uuid')
        require(isinstance(key, str) and UUID.fullmatch(key) and key not in nodes)
        side = row.get('isSidechain', False)
        require(isinstance(side, bool))
        nodes[key] = (row, lead + i)
        if not side and not row.get('teamName') and (compact_boundary(row) or (
                kind in ('user', 'assistant') and not any(row.get(flag) for flag in HIDDEN))):
            leaf = key
    def before_compaction(key, limit):
        # Walks the chain a compaction left behind, from the record its
        # boundary names. Only strictly earlier records are followed, so the
        # walk cannot cycle; an earlier compaction is crossed the same way.
        # Records before the parsed range are found by the bounded search.
        found = None
        while key is not None:
            require(isinstance(key, str) and UUID.fullmatch(key))
            if key in nodes:
                row, pos = nodes[key]
                below = min(start, pos)
            else:
                row, pos, below = find_record(fd, key, min(start, limit), low, sid)
            require(pos < limit)
            require(not row.get('isSidechain') and not row.get('teamName') and row.get('sessionId', sid) == sid)
            if row['type'] in ('assistant', 'user'):
                content = message_content(row, row['type'])
            if compact_boundary(row):
                if found is not None:
                    break
                key, limit = row.get('logicalParentUuid'), pos
                continue
            parent = row['parentUuid']
            if found is None and row['type'] == 'assistant' and not any(row.get(k) for k in HIDDEN):
                text = visible(content)
                if text.strip():
                    found = {'id': key, 'text': text}
            if found is not None and parent is not None and parent not in nodes:
                # Once a reply is found, the walk ends at an ancestor that
                # exists before the window within the search budget.
                require(isinstance(parent, str) and UUID.fullmatch(parent))
                find_record(fd, parent, below, low, sid)
                parent = None
            key, limit = parent, pos
        return found
    replies, crossing, floor, tip = [], None, start, leaf
    while leaf is not None:
        row, pos = nodes[leaf]
        require(not row.get('isSidechain') and not row.get('teamName') and row.get('sessionId', sid) == sid)
        if row['type'] in ('assistant', 'user'):
            content = message_content(row, row['type'])
        parent = row['parentUuid']
        if compact_boundary(row):
            # Neither parentUuid nor logicalParentUuid extends the active chain
            # across a compaction. With no reply after it, a viewer ('last')
            # still shows the newest reply from before it.
            require(parent is None or (isinstance(parent, str) and UUID.fullmatch(parent)))
            parent = None
            if mode == 'last' and not replies:
                crossing = (row.get('logicalParentUuid'), pos)
        elif parent is not None and parent not in nodes:
            # Only an ancestor before the parsed range may be absent, and it
            # must exist there. The tip a valid cursor already validated ends
            # the walk without a search; otherwise a bounded search finds it.
            require(floor > 0 and isinstance(parent, str) and UUID.fullmatch(parent))
            if known is not None and parent == known[0] and known[1] <= min(floor, boundary):
                parent = None
            else:
                found, found_end, found_start = find_record(fd, parent, floor, low, sid)
                if pending and found_end > boundary:
                    # A pending page is never acknowledged past an undelivered
                    # reply before the window: keep walking back to it.
                    nodes[parent], floor = (found, found_end), found_start
                else:
                    parent = None
        elif parent is not None:
            # Parents strictly precede children, so the walk cannot cycle.
            require(nodes[parent][1] < pos)
        if row['type'] == 'assistant' and not any(row.get(k) for k in HIDDEN):
            text = visible(content)
            if text.strip() and (mode == 'last' or pos > boundary):
                if mode != 'last' or not replies:
                    replies.append((pos, {'id': leaf, 'text': text}))
        leaf = parent
    if crossing is not None:
        # History before a compaction is read best effort: any record there
        # the active chain would reject, a link that leaves it, or an ancestor
        # missing or past the search budget shows none and fails nothing.
        try:
            found = before_compaction(*crossing)
        except Exception:
            found = None
        if found is not None:
            replies.append((end, found))
    # Validate the entire chain before selecting a page. Watches start at the
    # oldest unread reply; a viewer needs the newest suffix and the actual tip.
    encode = lambda obj: json.dumps(obj, separators=(',', ':'))
    def truncate(reply, room):
        # Cuts the text on a code-point boundary (a str index never splits a
        # surrogate pair in the escaped output) so the reply fits the room.
        text, low, high = reply['text'], 0, len(reply['text'])
        fit = lambda n: encode({'id': reply['id'], 'text': text[:n] + TRUNCATED})
        require(len(fit(0)) <= room)
        while low < high:
            mid = (low + high + 1) // 2
            if len(fit(mid)) <= room:
                low = mid
            else:
                high = mid - 1
        return fit(low)
    # The cursor carries the chain tip it validated, so a later poll whose new
    # records chain back to it ends there without searching the prefix. With
    # no new tip in an incremental parse, the previous one is still the tip.
    if tip is not None:
        known = (tip, nodes[tip][1])
    elif not incremental:
        known = None
    ancestry = '' if known is None else f':{known[0]}:{known[1]}'
    parts, size, offset = [], 0, end
    envelope = len(encode({'cursor': '', 'replies': []}))
    for pos, reply in (reversed(replies) if mode == 'since' else replies):
        part = encode(reply)
        candidate = pos if mode == 'since' else end
        added = len(part) + bool(parts)
        room = MAX_RESULT - envelope - len(cursor(end, candidate, sizing=True)) - size
        if added > room:
            require(parts or mode == 'since')  # Only a watch delivers a cut reply.
            if parts:
                break
            # A reply too large for a page alone is delivered cut, so it never
            # blocks the replies after it. It fills the page.
            part = truncate(reply, room)
            added = len(part)
        parts.append(part)
        size += added
        offset = candidate
    if len(parts) == len(replies) and envelope + len(cursor(end, end, sizing=True)) + size <= MAX_RESULT:
        offset = end
    if mode == 'recent':
        parts.reverse()
    out = '{"cursor":' + encode(cursor(end, offset)) + ',"replies":[' + ','.join(parts) + ']}'
    require(len(out) <= MAX_RESULT)
    after = os.fstat(fd)
    require(total == after.st_size and info.st_mtime_ns == after.st_mtime_ns)
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
