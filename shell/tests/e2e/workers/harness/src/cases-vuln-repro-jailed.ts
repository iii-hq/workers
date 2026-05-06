// Regression test for S-C1 (symlink-parent jail escape). Originally
// this case demonstrated the vuln by writing through a symlink whose
// target was outside host_root and observing the bytes land outside;
// after the fix in `shell/src/fs/host.rs` (canonicalize_with_fallback
// + lexical normalization of the non-existent tail), the same
// invocation must reject with S215. Runs only against
// `config-jailed.yaml`.

import {
  existsSync,
  mkdirSync,
  rmSync,
  symlinkSync,
} from 'node:fs';
import { join } from 'node:path';
import { randomUUID } from 'node:crypto';
import { expect, type CaseContext, type TestCase } from './cases.ts';
import { fsWriteStream } from './cases-fs-host.ts';

const JAIL_ROOT = '/private/tmp/iii-shell-jailed-root';

export const VULN_REPRO_JAILED_CASES: TestCase[] = [
  {
    name: 'vuln_repro_c1_symlink_parent_write_rejected_by_canonicalize_with_fallback',
    async run(ctx: CaseContext) {
      const escapeId = randomUUID();
      const externalDir = `/private/tmp/iii-shell-vuln-c1-escape-${escapeId}`;
      const escapeName = 'planted.txt';
      const externalTarget = join(externalDir, escapeName);

      mkdirSync(JAIL_ROOT, { recursive: true });
      mkdirSync(externalDir, { recursive: true });
      const linkInsideJail = join(JAIL_ROOT, `escape-${escapeId}`);
      try { rmSync(linkInsideJail, { force: true, recursive: false }); } catch { /* ok */ }
      symlinkSync(externalDir, linkInsideJail);

      const escapePath = join(linkInsideJail, escapeName);
      expect(!existsSync(escapePath), 'precondition: escape path must not exist yet');

      // FIX: validate_path now canonicalizes the longest existing
      // ancestor (which resolves the symlink to externalDir, outside
      // host_root) and lexically appends the non-existent tail. The
      // resulting path no longer starts_with(host_root) — engine
      // rejects with S215.
      let rejected = false;
      let observed = '';
      try {
        await fsWriteStream(ctx, {
          path: escapePath,
          mode: '0644',
          parents: false,
          bytes: 'should-be-rejected\n',
        });
      } catch (e: any) {
        rejected = true;
        observed = e?.message ?? String(e);
      }

      expect(
        rejected,
        `regression: write to symlinked-out-of-jail path was accepted (must reject with S215)`,
      );
      expect(
        observed.includes('S215'),
        `regression: rejection code wrong (want S215, got: ${observed})`,
      );
      expect(
        !existsSync(externalTarget),
        `regression: bytes still landed outside the jail at ${externalTarget}`,
      );

      try { rmSync(externalDir, { recursive: true, force: true }); } catch { /* ok */ }
      try { rmSync(linkInsideJail, { force: true }); } catch { /* ok */ }
    },
  },
  {
    // S-C1 second site: shell/src/fs/host.rs (write parents:true block).
    // The original lexical-fallback flaw existed both in validate_path
    // and in the parents:true defense-in-depth re-check. The fix to
    // validate_path closes the upstream side; the defense-in-depth now
    // uses host_root_canon (precomputed at HostFsBackend::new()) to
    // re-check the parent. This case exercises the parents:true path
    // explicitly: the external escape directory does NOT pre-exist, so
    // create_dir_all has to walk through the symlink, and the
    // defense-in-depth check is what catches the escape attempt.
    name: 'vuln_repro_c1b_symlink_parent_with_parents_true_rejected',
    async run(ctx: CaseContext) {
      const escapeId = randomUUID();
      const externalDir = `/private/tmp/iii-shell-vuln-c1b-escape-${escapeId}`;
      const escapeName = 'planted.txt';
      const externalTarget = join(externalDir, escapeName);

      mkdirSync(JAIL_ROOT, { recursive: true });
      // Intentionally do NOT create externalDir — parents:true should
      // try to mkdir through the symlink and the validation must
      // reject before any kernel mkdir call lands.
      const linkInsideJail = join(JAIL_ROOT, `escape-${escapeId}`);
      try { rmSync(linkInsideJail, { force: true, recursive: false }); } catch { /* ok */ }
      symlinkSync(externalDir, linkInsideJail);

      const escapePath = join(linkInsideJail, escapeName);

      let rejected = false;
      let observed = '';
      try {
        await fsWriteStream(ctx, {
          path: escapePath,
          mode: '0644',
          parents: true,
          bytes: 'should-be-rejected-via-parents-true\n',
        });
      } catch (e: any) {
        rejected = true;
        observed = e?.message ?? String(e);
      }

      expect(
        rejected,
        `regression: parents:true write to symlinked-out-of-jail path was accepted`,
      );
      expect(
        observed.includes('S215'),
        `regression: wrong rejection code (want S215, got: ${observed})`,
      );
      expect(
        !existsSync(externalTarget),
        `regression: bytes still landed outside the jail at ${externalTarget}`,
      );
      expect(
        !existsSync(externalDir),
        `regression: defense-in-depth let create_dir_all create ${externalDir} outside the jail`,
      );

      try { rmSync(externalDir, { recursive: true, force: true }); } catch { /* ok */ }
      try { rmSync(linkInsideJail, { force: true }); } catch { /* ok */ }
    },
  },
];
