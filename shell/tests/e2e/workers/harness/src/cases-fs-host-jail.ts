import { type CaseContext, type TestCase } from './cases.ts';

export const FS_HOST_JAIL_CASES: TestCase[] = [
  {
    name: 'fs_host_relative_path_rejected_s210',
    async run(ctx: CaseContext) {
      await ctx.expectError(
        () => ctx.call('shell::fs::stat', { path: 'relative/path' }),
        'S210',
      );
    },
  },
  {
    // 0.10.0 redaction: denylisted folds into S211, indistinguishable from
    // a missing path (S215 is exclusively a jail-confinement escape now).
    name: 'fs_host_denylist_etc_passwd_rejected_s211_redacted',
    async run(ctx: CaseContext) {
      await ctx.expectError(
        () => ctx.call('shell::fs::stat', { path: '/etc/passwd' }),
        'S211',
      );
    },
  },
  {
    name: 'fs_host_denylist_etc_shadow_rejected_s211_redacted',
    async run(ctx: CaseContext) {
      await ctx.expectError(
        () => ctx.call('shell::fs::read', { path: '/etc/shadow' }),
        'S211',
      );
    },
  },
  {
    name: 'fs_host_unknown_target_kind_rejected_s210',
    async run(ctx: CaseContext) {
      await ctx.expectError(
        () =>
          ctx.call('shell::fs::ls', {
            target: { kind: 'wat' },
            path: '/tmp',
          }),
        'S210',
      );
    },
  },
];
