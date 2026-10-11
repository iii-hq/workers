#!/usr/bin/env bash
# Stop only the processes boot.sh started (by pid file, never by name: other stacks run the same binaries).
# A pid counts only while its cwd is $RUN (boot.sh starts everything there), so a stale file never hits a
# recycled pid. TERM first, KILL whatever is still up after 10 s.
RUN=${RUN:?set RUN}
[ -f "$RUN/pids" ] || exit 0
mine=()
for p in $(tac "$RUN/pids"); do
  [ "$(readlink "/proc/$p/cwd" 2>/dev/null)" = "$RUN" ] && mine+=("$p")
done
alive=()
[ ${#mine[@]} -gt 0 ] && kill "${mine[@]}" 2>/dev/null
for _ in $(seq 20); do
  alive=()
  for p in "${mine[@]}"; do kill -0 "$p" 2>/dev/null && alive+=("$p"); done
  [ ${#alive[@]} -eq 0 ] && break
  sleep 0.5
done
[ ${#alive[@]} -gt 0 ] && kill -9 "${alive[@]}" 2>/dev/null
rm -f "$RUN/pids"
