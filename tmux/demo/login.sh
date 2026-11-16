#!/bin/bash
# demo/login.sh <engine-port> browser [demo-ok|demo-ok:<id>|deny]
# The browser on the worker's computer comes back from the sign-in page to
# the latest `claude auth login` started in container acct-demo-<engine-port>
# (its loopback listener, as with `browser: host` on "This Mac"): `demo-ok`
# (the default) signs in with no code, as the account's own person (or the
# one its email names), `demo-ok:<id>` as account <id>; `deny` is the person
# cancelling on the page: Claude Code ends (`failed`).
#
# The fake `claude auth login` (demo/files/claude) takes the codes the real
# one does, `<code>#<state>`:
#   demo-ok#demo-state           signs in (as above);
#   demo-ok:<id>#demo-state      signs in as <id> (on ~/.claude: <id> becomes global);
#   anything-else#x              refused as Anthropic refuses a code: Claude Code ends (`rejected`);
#   no `#`                       the worker answers `invalid_code` before Claude Code sees it.
set -euo pipefail
port=${1:?engine port}; what=${2:?browser}; code=${3:-demo-ok}
case "$what" in
  browser) ;;
  *) echo "usage: $0 <engine-port> browser [demo-ok|demo-ok:<id>|deny]" >&2; exit 2;;
esac
docker exec -u dev "acct-demo-$port" python3 -I -c '
import re, sys, urllib.parse, urllib.request
noted = open("/tmp/fake-claude/auth-login.url").read().strip()
redirect = urllib.parse.parse_qs(urllib.parse.urlparse(noted).query)["redirect_uri"][0]
if not re.fullmatch(r"http://localhost:\d+/callback", redirect):
    sys.exit("no sign-in with a loopback listener: %s" % redirect)
code = sys.argv[1]
query = "error=access_denied&state=demo-state" if code == "deny" else urllib.parse.urlencode(
    {"code": code, "state": "demo-state"})
url = redirect.replace("localhost", "127.0.0.1") + "?" + query
try:
    with urllib.request.urlopen(url, timeout=5) as answer:
        print(answer.status, answer.read().decode().strip())
except Exception as error:  # the listener is gone: that login is over
    sys.exit("the sign-in is not waiting any more (%s)" % error)
' "$code"
