#!/usr/bin/env python3
"""Offline gh api fixture. GH_TOKEN points to a private test-state file, not a token."""
import json
import os
import sys
import time
from pathlib import Path
from urllib.parse import parse_qs, urlsplit

p = Path(os.environ['GH_TOKEN'])
s = json.loads(p.read_text())
method, endpoint = sys.argv[3:5]
body = json.load(sys.stdin) if '--input' in sys.argv else None
s.setdefault('calls', []).append({'method': method, 'endpoint': endpoint, 'body': body})
path = endpoint.split('?')[0]
query = parse_qs(urlsplit(endpoint).query)
answer = None
failed = False
link = None
if method == 'POST' and path.endswith('/hooks'):
    failed = bool(s.get('fail_create'))
    if not failed or s.get('create_applied'):
        s['hook'] = {'id': 42, **body}
    answer = s.get('hook')
elif method == 'GET' and path.endswith('/hooks'):
    page = int(query.get('page', ['1'])[0])
    failed = bool(s.get('fail_list')) or page == s.get('fail_hook_page')
    pages = s.get('hook_pages', [[s['hook']] if s.get('hook') else []])
    answer = [s['hook'] if row == 'managed' else row for row in pages[page - 1]]
    if page < len(pages):
        link = f'https://api.github.com/{path}?per_page=100&page={page + 1}'
    link = s.get('hook_next_override', link)
elif method == 'GET' and path.endswith('/deliveries'):
    page = int(query.get('cursor', ['1'])[0])
    pages = s.get('delivery_pages', [[]])
    answer = pages[page - 1]
    if page < len(pages):
        # GitHub canonicalizes delivery links to the numeric repository route.
        link = f'https://api.github.com/repositories/123/hooks/42/deliveries?per_page=100&cursor={page + 1}'
    if page == s.get('pause_delivery_page'):
        p.with_suffix('.waiting').touch()
        deadline = time.monotonic() + 4
        while not p.with_suffix('.continue').exists() and time.monotonic() < deadline:
            time.sleep(0.005)
        failed = not p.with_suffix('.continue').exists()
elif method == 'POST' and path.endswith('/attempts'):
    failed = bool(s.get('fail_redeliver'))
elif '/hooks/42' in path and method == 'GET':
    failed = bool(s.get('fail_get_hook'))
    answer = s['hook']
elif '/hooks/42' in path and method == 'PATCH':
    failed = bool(s.get('fail_patch'))
    if not failed or s.get('patch_applied'):
        s['hook'].update(body)
    answer = s['hook']
elif '/hooks/42' in path and method == 'DELETE':
    failed = s.get('fail_delete', False)
    if not failed:
        s['hook'] = None
elif '/pulls/' in path:
    answer = s['pr']
elif '/check-runs?' in endpoint:
    answer = {'total_count': len(s.get('checks', [])), 'check_runs': s.get('checks', [])}
elif '/status?' in endpoint:
    answer = {'total_count': len(s.get('statuses', [])), 'statuses': s.get('statuses', [])}
else:
    failed = True
p.write_text(json.dumps(s))
if failed:
    sys.exit(1)
if '--include' in sys.argv:
    print('HTTP/2.0 200 OK')
    if link:
        print(f'Link: <{link}>; rel="next"')
    print()
print(json.dumps(answer))
