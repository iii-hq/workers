import { expect, expectEqual, type CaseContext, type TestCase } from './cases.ts';

// Real end-to-end coverage for the new sandbox-target path on
// shell::exec / shell::exec_bg.
//
// Unlike the fs-sandbox cases (which mock the engine's sandbox::fs::*
// triggers to test shell's wire shape only), these cases drive the
// *real* iii-sandbox worker. The flow is:
//
//   1. preflight: sandbox::create — boots a microVM, returns sandbox_id.
//      If iii-sandbox isn't installed or the host can't boot VMs (no
//      /dev/kvm on Linux, Intel Mac, Windows), this fails and the rest
//      of the suite logs SKIP and short-circuits — the operator gets a
//      clear "install iii-sandbox" message instead of a wall of red.
//   2. cases: each one calls shell::exec / shell::exec_bg with
//      target = { kind: 'sandbox', sandbox_id }. The engine routes the
//      sandbox::exec trigger to iii-sandbox, which runs the command in
//      the microVM and returns the real result.
//   3. teardown: the last case calls sandbox::stop on the shared
//      sandbox.
//
// To run: the engine config (tests/e2e/config.yaml) must list
// `iii-sandbox` under workers, and the host must support
// hardware virtualization. See README §Sandbox-backed exec for the
// host-virt requirement.

interface SandboxState {
  status: 'unknown' | 'available' | 'unavailable';
  sandboxId: string | null;
  skipReason: string;
}

const sandbox: SandboxState = {
  status: 'unknown',
  sandboxId: null,
  skipReason: '',
};

/**
 * Boot a sandbox once (cached on the module) and return its id; record
 * the suite as unavailable on failure so subsequent cases can skip
 * cleanly without each one re-trying sandbox::create.
 */
async function ensureSandbox(ctx: CaseContext): Promise<string | null> {
  if (sandbox.status === 'available') return sandbox.sandboxId;
  if (sandbox.status === 'unavailable') return null;
  try {
    const r = await ctx.call('sandbox::create', {
      image: 'python',
      cpus: 1,
      memory_mb: 256,
    });
    sandbox.sandboxId = r.sandbox_id;
    sandbox.status = 'available';
    return sandbox.sandboxId;
  } catch (e: unknown) {
    const msg = e instanceof Error ? e.message : String(e);
    sandbox.status = 'unavailable';
    sandbox.skipReason = `iii-sandbox not reachable or host lacks virt: ${msg}`;
    return null;
  }
}

/**
 * Standard preamble for every case in this file: returns the shared
 * sandbox_id, or `null` after logging a skip. When `null`, the case
 * should `return` immediately — the runner records it as PASS but the
 * console output makes the skip visible to the operator.
 */
async function maybeSandbox(ctx: CaseContext, caseName: string): Promise<string | null> {
  const id = await ensureSandbox(ctx);
  if (!id) {
    console.log(`[harness] SKIP :: ${caseName} — ${sandbox.skipReason}`);
  }
  return id;
}

export const EXEC_SANDBOX_CASES: TestCase[] = [
  {
    name: 'exec_sandbox_real_echo_runs_in_vm',
    async run(ctx: CaseContext) {
      const sandbox_id = await maybeSandbox(ctx, 'exec_sandbox_real_echo_runs_in_vm');
      if (!sandbox_id) return;
      const r = await ctx.call('shell::exec', {
        command: 'echo',
        args: ['hi'],
        target: { kind: 'sandbox', sandbox_id },
      });
      expectEqual(r.exit_code, 0, 'exit_code from in-VM echo');
      expectEqual(r.stdout, 'hi\n', 'stdout from in-VM echo');
      expectEqual(r.timed_out, false, 'timed_out');
      expectEqual(r.stdout_truncated, false, 'sandbox path forces stdout_truncated false');
      expectEqual(r.stderr_truncated, false, 'sandbox path forces stderr_truncated false');
      expect(typeof r.duration_ms === 'number' && r.duration_ms >= 0, 'duration_ms present');
    },
  },
  {
    name: 'exec_sandbox_captures_stderr_separately',
    async run(ctx: CaseContext) {
      const sandbox_id = await maybeSandbox(ctx, 'exec_sandbox_captures_stderr_separately');
      if (!sandbox_id) return;
      const r = await ctx.call('shell::exec', {
        command: 'sh',
        args: ['-c', 'echo err 1>&2'],
        target: { kind: 'sandbox', sandbox_id },
      });
      expectEqual(r.exit_code, 0, 'exit_code');
      expectEqual(r.stdout, '', 'stdout empty');
      expectEqual(r.stderr, 'err\n', 'stderr captured from VM');
    },
  },
  {
    name: 'exec_sandbox_propagates_non_zero_exit',
    async run(ctx: CaseContext) {
      const sandbox_id = await maybeSandbox(ctx, 'exec_sandbox_propagates_non_zero_exit');
      if (!sandbox_id) return;
      const r = await ctx.call('shell::exec', {
        command: 'false',
        target: { kind: 'sandbox', sandbox_id },
      });
      expectEqual(r.exit_code, 1, 'exit_code from in-VM `false`');
      expectEqual(r.timed_out, false, 'timed_out');
    },
  },
  {
    name: 'exec_sandbox_timed_out_when_command_exceeds_clamped_timeout',
    async run(ctx: CaseContext) {
      const sandbox_id = await maybeSandbox(
        ctx,
        'exec_sandbox_timed_out_when_command_exceeds_clamped_timeout',
      );
      if (!sandbox_id) return;
      // tests/e2e/config.yaml sets max_timeout_ms: 5000 and
      // default_timeout_ms: 1500. `sleep 30` will exceed both no
      // matter what the caller asks for; the clamp ensures the VM-
      // side timeout is bounded. We give the round-trip a generous
      // 8s wall-clock wait below.
      const r = await ctx.call('shell::exec', {
        command: 'sleep',
        args: ['30'],
        timeout_ms: 60_000, // clamped to max_timeout_ms (5s) before forwarding
        target: { kind: 'sandbox', sandbox_id },
      });
      expectEqual(r.timed_out, true, 'timed_out flag set by sandbox::exec');
    },
  },
  {
    name: 'exec_sandbox_bad_sandbox_id_returns_engine_error',
    async run(ctx: CaseContext) {
      const sandbox_id = await maybeSandbox(
        ctx,
        'exec_sandbox_bad_sandbox_id_returns_engine_error',
      );
      if (!sandbox_id) return;
      // S001/S002: the engine returns one of these for an unknown
      // sandbox_id. Don't pin the exact code (the daemon picks; both
      // are acceptable per the spec's user-facing S-code table).
      const fakeId = '00000000-1111-2222-3333-444444444444';
      await ctx.expectError(
        () =>
          ctx.call('shell::exec', {
            command: 'echo',
            args: ['nope'],
            target: { kind: 'sandbox', sandbox_id: fakeId },
          }),
        'S00',
      );
    },
  },
  {
    name: 'exec_sandbox_allowlist_still_enforced',
    async run(ctx: CaseContext) {
      const sandbox_id = await maybeSandbox(ctx, 'exec_sandbox_allowlist_still_enforced');
      if (!sandbox_id) return;
      // The shell allowlist gates argv[0] BEFORE backend dispatch for
      // both targets. `nmap` is not in the e2e allowlist
      // (echo/ls/cat/sleep/pwd/printf/sh/false/true/env), so the call
      // must fail with an allowlist error WITHOUT reaching the VM.
      // (We can't directly observe "VM not hit" the way we can with
      // mocks; the assertion is just that the shell denies the call.)
      await ctx.expectError(
        () =>
          ctx.call('shell::exec', {
            command: 'nmap',
            args: ['-A', 'localhost'],
            target: { kind: 'sandbox', sandbox_id },
          }),
        'allowlist',
      );
    },
  },
  {
    name: 'exec_bg_sandbox_lifecycle_finishes',
    async run(ctx: CaseContext) {
      const sandbox_id = await maybeSandbox(ctx, 'exec_bg_sandbox_lifecycle_finishes');
      if (!sandbox_id) return;
      const start = await ctx.call('shell::exec_bg', {
        command: 'echo',
        args: ['bg-ok'],
        target: { kind: 'sandbox', sandbox_id },
      });
      expect(typeof start.job_id === 'string' && start.job_id.startsWith('job-'),
        `job_id shape: ${JSON.stringify(start.job_id)}`,
      );
      // Poll status until the spawned tokio task finalizes the
      // JobRecord. echo is fast but the VM round-trip dominates;
      // budget 5s with 100ms polling.
      const deadline = Date.now() + 5_000;
      let finalStatus: any = null;
      while (Date.now() < deadline) {
        const s = await ctx.call('shell::status', { job_id: start.job_id });
        if (s.job.status !== 'running') {
          finalStatus = s;
          break;
        }
        await ctx.sleep(100);
      }
      expect(finalStatus !== null, 'sandbox bg job finalized within 5s');
      expectEqual(finalStatus.job.status, 'finished', 'sandbox bg job ends Finished');
      expectEqual(finalStatus.job.exit_code, 0, 'exit_code captured into JobRecord');
      expectEqual(finalStatus.job.stdout, 'bg-ok\n', 'stdout captured');
    },
  },
  {
    name: 'shell_kill_on_sandbox_job_returns_caveat_reason',
    async run(ctx: CaseContext) {
      const sandbox_id = await maybeSandbox(
        ctx,
        'shell_kill_on_sandbox_job_returns_caveat_reason',
      );
      if (!sandbox_id) return;
      // Start a long-running sandbox job (sleep 30) capped at 1s so the
      // server-side timeout fires fast. Kill it while the trigger is
      // still in flight. shell::kill must:
      //   - return killed: true
      //   - include the caveat string ("sandbox::exec ... timeout_ms")
      //   - flip status to killed immediately
      // The in-VM sleep keeps running until its 1s server-side timeout
      // — proving the documented status-level-only cancel semantics.
      const start = await ctx.call('shell::exec_bg', {
        command: 'sleep',
        args: ['30'],
        timeout_ms: 1000,
        target: { kind: 'sandbox', sandbox_id },
      });
      // Brief wait so the tokio task is parked inside fwd.trigger.
      await ctx.sleep(150);
      const killResp = await ctx.call('shell::kill', { job_id: start.job_id });
      expectEqual(killResp.killed, true, 'kill reports killed=true for sandbox job');
      expectEqual(killResp.status, 'killed', 'status flipped to killed immediately');
      expect(typeof killResp.reason === 'string' && killResp.reason.includes('sandbox::exec'),
        `kill reason mentions sandbox::exec: ${killResp.reason}`,
      );
      expect(killResp.reason.includes('timeout_ms'),
        `kill reason mentions timeout_ms: ${killResp.reason}`,
      );
      // Status reads `killed` immediately after the kill call, before
      // the in-VM sleep's server-side timeout would expire.
      const midStatus = await ctx.call('shell::status', { job_id: start.job_id });
      expectEqual(midStatus.job.status, 'killed', 'status reads killed before late completion');
      // Wait for the late completion to arrive (1s server-side timeout
      // plus margin). The already_killed guard MUST preserve
      // status=killed.
      await ctx.sleep(1_500);
      const lateStatus = await ctx.call('shell::status', { job_id: start.job_id });
      expectEqual(
        lateStatus.job.status,
        'killed',
        'late completion does NOT overwrite Killed status',
      );
    },
  },
  {
    name: 'sandbox_stop_tears_down_shared_sandbox',
    async run(ctx: CaseContext) {
      // Teardown for the suite. Runs last so prior cases all reuse
      // the same sandbox; stops it explicitly so the VM doesn't leak
      // until the engine's idle reaper kicks in (300s default).
      const sandbox_id = await maybeSandbox(ctx, 'sandbox_stop_tears_down_shared_sandbox');
      if (!sandbox_id) return;
      const r = await ctx.call('sandbox::stop', { sandbox_id, wait: true });
      expectEqual(r.sandbox_id, sandbox_id, 'sandbox::stop echoes the id');
      expect(r.stopped === true, `sandbox::stop reports stopped=true: ${JSON.stringify(r)}`);
      // Mark the suite consumed so any case ordering issues don't
      // accidentally try to reuse the now-stopped sandbox.
      sandbox.status = 'unavailable';
      sandbox.skipReason = 'sandbox already stopped by teardown';
    },
  },
];
