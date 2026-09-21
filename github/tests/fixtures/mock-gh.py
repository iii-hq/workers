#!/usr/bin/env python3
"""Offline gh api fixture. GH_TOKEN points to a private test-state file, not a token."""
import json
import os
import sys
from pathlib import Path
p = Path(os.environ['GH_TOKEN'])
s = json.loads(p.read_text())
method, endpoint = sys.argv[3:5]
body = json.load(sys.stdin) if '--input' in sys.argv else None
s.setdefault('calls', []).append({'method': method, 'endpoint': endpoint, 'body': body})
answer = None
failed = False
if method == 'POST' and endpoint.endswith('/hooks'):
    s['hook'] = {'id': 42, **body}
    answer = s['hook']
elif '/hooks/42' in endpoint and method == 'GET':
    answer = s['hook']
elif '/hooks/42' in endpoint and method == 'PATCH':
    s['hook'].update(body)
    answer = s['hook']
elif '/hooks/42' in endpoint and method == 'DELETE':
    failed = s.get('fail_delete', False)
    if not failed:
        s['hook'] = None
elif '/pulls/' in endpoint:
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
print(json.dumps(answer))
