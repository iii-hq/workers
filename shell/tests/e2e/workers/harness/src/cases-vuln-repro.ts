// Regression tests for the security findings raised in the
// `feat/shell-e2e-harness` review. Originally these cases passed
// _because the vuln was present_; after the fix they assert the
// post-fix behavior (error code, redaction, opt-in flag). Names start
// with `vuln_repro_` for grep continuity.
//
// Coverage in this file (default config = host_root: null,
// allow_unjailed: true):
//   - S-H1: chmod -R no longer rewrites symlink targets
//   - S-H2: unjailed mode requires explicit allow_unjailed: true (the
//     test config sets it, demonstrating the opt-in surface)
//   - S-H3: denylist is advisory; literal-form patterns reject, but
//     trivially-bypassable. Documented as such — the test pins both
//     halves of the contract.
//   - S-H4: shell::list returns summary records only; argv/stdout are
//     reachable only via shell::status <job_id> (the UUID is the cap).
//
// S-C1 (symlink-parent escape) is tested in `cases-vuln-repro-jailed.ts`
// against `config-jailed.yaml` (host_root set).

import { mkdtempSync, mkdirSync, writeFileSync, statSync, symlinkSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { randomUUID } from 'node:crypto';
import { expect, expectEqual, type CaseContext, type TestCase } from './cases.ts';
import { fsReadStream, fsWriteStream } from './cases-fs-host.ts';

function newWorkdir(prefix: string): string {
  return mkdtempSync(join(tmpdir(), `iii-shell-vuln-${prefix}-`));
}

function modeOctal(path: string): string {
  return (statSync(path).mode & 0o777).toString(8).padStart(3, '0');
}

export const VULN_REPRO_CASES: TestCase[] = [
  {
    // S-H1: shell/src/fs/host.rs (chmod handler).
    // Fix: the recursive walk skips entries whose file_type is a
    // symlink, so chmod(2)/chown(2) never deref to a target outside
    // the walk root. The link itself remains untouched (lchmod isn't
    // portable; leaving the link mode alone is the safe default).
    // Companion case below pins the related "walk root is a symlink"
    // surprise: that one returns S212 instead of a silent no-op.
    name: 'vuln_repro_h1_chmod_recursive_skips_symlinks',
    async run(ctx: CaseContext) {
      const root = newWorkdir('h1');
      const sub = join(root, 'sub');
      const target = join(newWorkdir('h1-target'), 'external-file');
      mkdirSync(sub, { recursive: true });
      writeFileSync(target, 'guarded\n', { mode: 0o600 });
      symlinkSync(target, join(sub, 'link-to-external'));

      const before = modeOctal(target);
      expectEqual(before, '600', 'precondition: external target starts at 0600');

      await ctx.call('shell::fs::chmod', {
        path: sub,
        mode: '0777',
        recursive: true,
      });

      const after = modeOctal(target);
      // FIX: external target's mode unchanged because the recursive
      // walk skipped the symlink entry. A regression here means the
      // skip-symlinks branch in chmod() was reverted or weakened.
      expectEqual(
        after,
        '600',
        `regression: external target mode changed via symlink (got ${after}, must stay 600)`,
      );
      rmSync(root, { recursive: true, force: true });
      rmSync(target, { force: true });
    },
  },
  {
    // S-H1 follow-up: walk root is itself a symlink. Without the
    // explicit S212, the per-entry skip-symlink branch would silently
    // skip the only entry walkdir yields and the call would return
    // {updated: 0} masquerading as success. The fix in chmod() checks
    // symlink_metadata on the root before entering the loop and
    // refuses with S212 (wrong file type).
    name: 'vuln_repro_h1b_chmod_recursive_on_symlink_root_returns_s212',
    async run(ctx: CaseContext) {
      const root = newWorkdir('h1b');
      const targetDir = newWorkdir('h1b-target');
      const link = join(root, 'link-to-dir');
      symlinkSync(targetDir, link);

      let observed = '';
      let rejected = false;
      try {
        await ctx.call('shell::fs::chmod', {
          path: link,
          mode: '0777',
          recursive: true,
        });
      } catch (e: any) {
        rejected = true;
        observed = e?.message ?? String(e);
      }
      expect(
        rejected,
        `regression: recursive chmod on a symlink path silently succeeded`,
      );
      expect(
        observed.includes('S212'),
        `regression: wrong rejection code (want S212, got: ${observed})`,
      );
      rmSync(root, { recursive: true, force: true });
      rmSync(targetDir, { recursive: true, force: true });
    },
  },
  {
    // S-H2: shell/src/config.rs (validate_fs_jail).
    // Fix: the worker refuses to start when host_root is null AND
    // fs.allow_unjailed is false. The test config sets allow_unjailed:
    // true (test harness writes to OS tmpdirs scattered across the
    // FS), so the writes-anywhere behavior is preserved as the
    // explicit opt-in. Verifying the refuse-to-start path requires a
    // negative startup test — covered in unit tests via
    // ShellConfig::validate_fs_jail. Here we just confirm the opt-in
    // surface still functions.
    name: 'vuln_repro_h2_unjailed_mode_is_opt_in_via_allow_unjailed',
    async run(ctx: CaseContext) {
      const path = `/var/tmp/iii-shell-vuln-h2-${randomUUID()}`;

      const writeRes = await fsWriteStream(ctx, {
        path,
        mode: '0644',
        bytes: 'unjailed-by-explicit-opt-in\n',
      });
      expect(
        writeRes.bytes_written > 0,
        `regression: opt-in unjailed write failed (path=${path}, got=${writeRes.bytes_written})`,
      );

      const readRes = await fsReadStream(ctx, { path });
      expectEqual(
        readRes.data.toString('utf8'),
        'unjailed-by-explicit-opt-in\n',
        'round-trip',
      );

      rmSync(path, { force: true });
    },
  },
  {
    // S-H3: shell/src/config.rs:148-171 + shell/config.yaml + README.md.
    // The denylist is advisory; the README and the shipped config
    // comment now state this explicitly. We pin both halves of the
    // contract so a future "let's tighten it" attempt is forced to
    // either invalidate this test or actually upgrade the boundary.
    name: 'vuln_repro_h3_denylist_is_advisory_literal_match_rejects_construction_bypasses',
    async run(ctx: CaseContext) {
      // (a) Literal form is still rejected — the tripwire works for
      // honest mistakes.
      await ctx.expectError(
        () =>
          ctx.call('shell::exec', {
            command: 'echo',
            args: ['harness_denylist_marker'],
          }),
        'denylist',
      );

      // (b) Bypass via shell-variable construction still works,
      // because regex against argv.join(" ") was never a real
      // boundary. If a future change blocks this, the README's
      // advisory-only framing is also stale and needs updating.
      const r = await ctx.call('shell::exec', {
        command: 'sh',
        args: ['-c', 'h=harness;d=denylist_marker;echo ${h}_${d}'],
      });
      expectEqual(r.exit_code, 0, 'bypass exec succeeded');
      expectEqual(
        r.stdout,
        'harness_denylist_marker\n',
        'regression-or-rewrite: denylist behavior changed; reconcile README + this test',
      );
    },
  },
  {
    // S-H4: shell/src/jobs.rs + shell/src/functions/list.rs +
    // shell/src/functions/types.rs.
    // Fix: shell::list returns JobSummary records (id, status,
    // timestamps, exit_code, *_truncated) — argv/stdout/stderr
    // omitted. The full record stays reachable through
    // shell::status <job_id>, where the UUID job_id is the
    // unguessable capability for that record.
    name: 'vuln_repro_h4_list_redacts_argv_and_stdout_status_keeps_full_record',
    async run(ctx: CaseContext) {
      const fakeToken = `vuln-repro-h4-${randomUUID()}`;
      const spawned = await ctx.call('shell::exec_bg', {
        command: 'sh',
        args: ['-c', `printf '%s\\n' "${fakeToken}"`],
      });
      const jobId = spawned.job_id as string;

      let finished = false;
      for (let i = 0; i < 20 && !finished; i++) {
        await ctx.sleep(50);
        const s = await ctx.call('shell::status', { job_id: jobId });
        finished = s?.job?.status === 'finished';
      }
      expect(finished, 'job finished within 1s');

      const list = await ctx.call('shell::list', {});
      const records = (list.jobs ?? []) as Array<Record<string, any>>;
      const ours = records.find((j) => j.id === jobId);
      expect(!!ours, 'spawned job present in list');

      // FIX assertions — argv and stdout/stderr must NOT appear in
      // the summary shape. If a regression re-adds them, the
      // structural cross-caller leak is back.
      expect(
        ours!.argv === undefined,
        `regression: shell::list still leaks argv (got ${JSON.stringify(ours!.argv)})`,
      );
      expect(
        ours!.stdout === undefined,
        `regression: shell::list still leaks stdout (got ${JSON.stringify(ours!.stdout)})`,
      );
      expect(
        ours!.stderr === undefined,
        `regression: shell::list still leaks stderr (got ${JSON.stringify(ours!.stderr)})`,
      );

      // Useful summary fields stay visible.
      expectEqual(ours!.status, 'finished', 'summary keeps status');
      expectEqual(ours!.exit_code, 0, 'summary keeps exit_code');
      expect(typeof ours!.started_at_ms === 'number', 'summary keeps started_at_ms');

      // Full record (with argv + stdout) still reachable via
      // shell::status using the job_id capability.
      const status = await ctx.call('shell::status', { job_id: jobId });
      const job = status.job;
      expect(
        Array.isArray(job.argv) && JSON.stringify(job.argv).includes(fakeToken),
        `shell::status returns argv with the token (cap-gated by job_id)`,
      );
      expect(
        String(job.stdout ?? '').includes(fakeToken),
        `shell::status returns full stdout (cap-gated by job_id)`,
      );
    },
  },
];
