#!/usr/bin/env bash
# Stop only the processes boot.sh started (by pid file, never by name: other stacks run the same binaries).
RUN=${RUN:?set RUN}
[ -f "$RUN/pids" ] && tac "$RUN/pids" | xargs -r kill 2>/dev/null
rm -f "$RUN/pids"
