#!/bin/bash
# demo/seed.sh <engine-port>: the artboards' sessions in container acct-demo-<engine-port>.
set -euo pipefail
port=${1:?engine port}
docker exec -u dev "acct-demo-$port" /opt/demo/seed-inside.sh
