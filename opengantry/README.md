# opengantry

OpenGantry gates promote-class calls on the iii bus. Install the worker, wire `gantry::middleware` on your governed listener, and agents cannot merge, deploy, or publish until a prior `gantry::verify` pass mints a verdict token.

## Install

```bash
iii worker add opengantry
```

## Skills

```bash
npx skills add iii-hq/workers --skill opengantry
```

## Quickstart

From zero to a fail-closed promote on the governed port:

```bash
iii worker add opengantry
iii   # starts the engine + worker
```

Add the governed listener block from [Configuration](#configuration) to `~/.iii/config.yaml`, restart `iii`, then call any promote-class function on the governed port:

```js
import { registerWorker } from 'iii-sdk';

const iii = registerWorker('ws://127.0.0.1:49135', { workerName: 'demo' });

const result = await iii.trigger({
  function_id: 'myapp::deploy',
  payload: { branch: 'main' },
  context: {
    msn_id: 'MSN-0001',
    worktree_path: '/path/to/repo',
  },
});

console.log(result);
```

Without a verdict token from `gantry::verify`, the middleware returns:

```json
{
  "status": "failed",
  "findings": [
    { "failed_gate": "gate", "resolution_hint": "promote refused: no valid verdict token" }
  ]
}
```

Initialize OpenGantry in the repo you want governed (`gantry init`), run `gantry::verify` for the active mission, then retry the promote call with the verdict token in `context` or `payload`.

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

`worktree_path` / `repo_root` in trigger context must be absolute. Leases persist at `<repo>/.gitagent/leases.json`.
