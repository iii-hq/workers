# opengantry

OpenGantry governance on the iii bus: scan your project's local `workers/` tree against iii bundle practices, then run `gantry::verify` on the git repo before promote-class calls go through. Wire `gantry::middleware` on the governed listener so agents cannot bypass verify.

## Install

```bash
iii worker add opengantry
```

## Skills

```bash
npx skills add iii-hq/workers --skill opengantry
```

## Quickstart

`iii worker add` registers the worker and starts a libkrun sandbox. That sandbox only mounts the worker bundle at `/workspace`, so it cannot read your host git repo. To verify a host `repo_root`, stop the sandbox and run the extracted bundle as a **host process** (verified on iii 0.22.0):

```bash
iii worker add opengantry
iii worker stop -y opengantry

export III_URL=ws://127.0.0.1:49134
export OTEL_ENABLED=false
cd ~/.iii/workers-bundle/opengantry
node ./index.mjs
```

In another terminal, initialize OpenGantry in the repo you want governed, then call verify with an absolute `repo_root`:

```bash
gantry init
```

```js
import { registerWorker } from 'iii-sdk';

const iii = registerWorker('ws://127.0.0.1:49134', { workerName: 'caller' });

const result = await iii.trigger({
  function_id: 'gantry::verify',
  payload: {
    repo_root: '/absolute/path/to/your/repo',
    msn_id: 'MSN-0001',
    mission_rel_path: '.gitagent/missions/MSN-0001.yaml',
  },
});

console.log(result);
```

`gantry::verify` scans `<repo_root>/workers` first. Findings fail verify even when the GXT mission gate would pass.

After every engine restart, the sandbox worker respawns from `config.yaml`. Run `iii worker stop -y opengantry` again before starting the host process so triggers are not routed to the sandbox.

## Configuration

On the governed listener, wire middleware and RBAC hooks (replace `session::auth` with your IdP worker):

```yaml
workers:
  - name: opengantry
  - name: iii-worker-manager
    config:
      host: 0.0.0.0
      port: 49135
      middleware_function_id: gantry::middleware
      rbac:
        auth_function_id: session::auth
        on_function_registration_function_id: gantry::on-function-registration
        on_trigger_registration_function_id: gantry::on-trigger-registration
        on_trigger_type_registration_function_id: gantry::on-trigger-type-registration
```

`repo_root` / `worktree_path` must be absolute. Leases live at `<repo>/.gitagent/leases.json`.

## Functions

| Function | Purpose |
| --- | --- |
| `gantry::verify` | Scan local `workers/`, then `verifyMission` |
| `gantry::middleware` | Governed-port gate; promote-class needs a prior verify pass |
| `gantry::on-function-registration` | Block `gantry::*` squatting |
| `gantry::on-trigger-registration` | Block triggers bound to `gantry::` |
| `gantry::on-trigger-type-registration` | Always denied |
