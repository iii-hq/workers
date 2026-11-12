# The tmux worker's demo container (agent-accounts §3.10): its own tmux
# server, a fake Claude Code and Codex on PATH, fake accounts and a mock of
# Claude Code's usage endpoint. The worker binary is mounted at run time
# (demo/run.sh), so one image serves every build of the branch.
FROM ubuntu:24.04
ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update -qq \
 && apt-get install -y -qq tmux procps locales python3 ca-certificates curl tzdata >/dev/null \
 && sed -i 's/^# *en_US.UTF-8/en_US.UTF-8/' /etc/locale.gen && locale-gen >/dev/null \
 && rm -rf /var/lib/apt/lists/* \
 && (id -u ubuntu >/dev/null 2>&1 && userdel -r ubuntu || true) \
 && useradd -m -u 1000 -s /bin/bash dev
ENV LANG=en_US.UTF-8 LC_ALL=en_US.UTF-8
# What every demo script checks (with /.dockerenv) before it writes a file or
# talks to tmux: they never run on the host.
ENV DEMO_CONTAINER=1
COPY files/claude files/codex files/claude-state /usr/local/bin/
COPY files/entrypoint.sh files/mock_usage.py files/setup_demo.py files/seed-inside.sh /opt/demo/
RUN chmod 755 /usr/local/bin/claude /usr/local/bin/codex /usr/local/bin/claude-state /opt/demo/*.sh /opt/demo/*.py
USER dev
WORKDIR /home/dev
ENTRYPOINT ["/opt/demo/entrypoint.sh"]
