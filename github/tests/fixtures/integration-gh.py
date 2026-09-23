#!/usr/bin/python3
"""Offline gh api contract fixture. GH_TOKEN is a temp state path, never a token.

Unknown endpoints/methods fail closed. No network, subprocesses or real hooks.
"""
import fcntl
import json
import os
import sys
from urllib.parse import parse_qs, urlsplit

assert sys.argv[1:3] == ['api', '--method'], sys.argv
method, endpoint = sys.argv[3:5]
path = urlsplit(endpoint).path
query = parse_qs(urlsplit(endpoint).query)
next_link = None
body = json.load(sys.stdin) if '--input' in sys.argv else None
with open(os.environ['GH_TOKEN'], 'r+', encoding='utf-8') as stream:
    fcntl.flock(stream, fcntl.LOCK_EX)
    state = json.load(stream)
    state.setdefault('calls', []).append({'method': method, 'endpoint': endpoint, 'body': body})
    prefix = 'repos/owner/repo/'
    answer = None
    failed = False
    if not path.startswith(prefix):
        failed = True
    elif path in (prefix + 'pulls/1', prefix + 'pulls/2') and method == 'GET':
        answer = state['prs'][path.rsplit('/', 1)[1]]
    # Specific deliveries route MUST precede the generic owned-hook route.
    elif path == prefix + 'hooks/42/deliveries' and method == 'GET':
        assert 'page' not in query, 'GitHub deliveries reject numeric page offsets'
        assert '--include' in sys.argv, 'delivery pagination requires Link headers'
        assert set(query) <= {'per_page', 'cursor'}
        assert query.get('per_page') == ['100']
        pages = state.get('delivery_pages', [state.get('deliveries', [])])
        cursor = query.get('cursor', [''])[0]
        index = 0 if not cursor else int(cursor.removeprefix('opaque-').removesuffix('='))
        answer = pages[index]
        if index + 1 < len(pages):
            # Keep accepting only the original repo endpoint: the worker must
            # reuse its verified base rather than follow this numeric alias.
            next_link = f'https://api.github.com/repositories/123/hooks/42/deliveries?per_page=100&cursor=opaque-{index + 1}%3D'
    elif path.startswith(prefix + 'hooks/42/deliveries/') and path.endswith('/attempts') and method == 'POST':
        answer = {}
    elif path == prefix + 'hooks' and method == 'POST':
        assert not state.get('hook'), 'duplicate hook creation'
        state['hook'] = {'id': 42, **body}
        answer = state['hook']
    elif path == prefix + 'hooks' and method == 'GET':
        answer = [state['hook']] if state.get('hook') else []
    elif path == prefix + 'hooks/42' and method == 'GET':
        answer = state.get('hook')
        failed = answer is None
    elif path == prefix + 'hooks/42' and method == 'PATCH':
        assert state.get('hook'), 'PATCH without owned hook'
        state['hook'].update(body)
        answer = state['hook']
    elif path == prefix + 'hooks/42' and method == 'DELETE':
        failed = state.get('fail_delete', False)
        if not failed:
            state['hook'] = None
    elif path.startswith(prefix + 'commits/') and method == 'GET':
        if path.endswith('/check-runs'):
            answer = {'total_count': 0, 'check_runs': []}
        elif path.endswith('/status'):
            answer = {'state': 'pending', 'total_count': 0, 'statuses': []}
        elif path.endswith('/statuses'):
            answer = []
        else:
            failed = True
    elif path == prefix + 'actions/runs' and method == 'GET':
        answer = {'total_count': 0, 'workflow_runs': []}
    else:
        failed = True
    stream.seek(0)
    json.dump(state, stream, ensure_ascii=False)
    stream.truncate()
if failed:
    print(f'unsupported or injected failure: {method} {endpoint}', file=sys.stderr)
    sys.exit(1)
if '--include' in sys.argv:
    print('HTTP/2.0 200 OK')
    if next_link:
        print(f'Link: <{next_link}>; rel="next"')
    print()
print(json.dumps(answer, ensure_ascii=False))
