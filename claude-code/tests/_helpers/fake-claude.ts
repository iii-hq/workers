/**
 * Claude Code's own plugin commands, as far as the worker uses them:
 * `claude plugin list --json`, `claude plugin marketplace list|add`,
 * `claude plugin install|enable`, plus `pwd -P` for the real folder. The
 * state is what Claude Code keeps per user: the known marketplaces, every
 * install with its scope and project.
 */

export type Install = {
  id: string;
  scope: 'user' | 'project' | 'local';
  enabled: boolean;
  projectPath?: string;
};

export type ClaudeRun = { program: string; args: string[]; cwd: string };

export type FakeClaude = {
  marketplaces: string[];
  installs: Install[];
  runs: ClaudeRun[];
  /** Symlinked folders: what `pwd -P` answers for them. */
  realpaths: Record<string, string>;
  /** Folders that do not exist on the host. */
  missing: Set<string>;
  /** Make one subcommand fail (`'plugin list'`, `'plugin install'`, ...). */
  fail: Set<string>;
  /** Make `plugin list` print something that is not JSON. */
  garbledList: boolean;
  run: (
    program: string,
    args: string[],
    cwd: string,
  ) => Promise<{
    stdout: string;
    stderr: string;
    exit_code: number;
  }>;
  /** The argv of every claude call, as `plugin list` / `plugin install ...`. */
  claudeCalls: () => string[];
};

export const CLAUDE = '/opt/claude/bin/claude';

/** The terminal host resolves a path; a headless turn may run plain `claude`. */
const isClaude = (program: string) => program === 'claude' || program.endsWith('/claude');

export function fakeClaude(initial: Partial<Pick<FakeClaude, 'marketplaces' | 'installs'>> = {}) {
  const fake: FakeClaude = {
    marketplaces: [...(initial.marketplaces ?? ['iii'])],
    installs: [...(initial.installs ?? [])],
    runs: [],
    realpaths: {},
    missing: new Set(),
    fail: new Set(),
    garbledList: false,
    claudeCalls: () => fake.runs.filter((r) => isClaude(r.program)).map((r) => r.args.join(' ')),
    run: async (program, args, cwd) => {
      fake.runs.push({ program, args: [...args], cwd });
      const ok = (stdout: string) => ({ stdout, stderr: '', exit_code: 0 });
      const err = (stderr: string, exit_code = 1) => ({ stdout: '', stderr, exit_code });
      if (fake.missing.has(cwd)) return err(`cd: ${cwd}: No such file or directory`, 2);
      const real = fake.realpaths[cwd] ?? cwd;
      if (program === 'pwd') return ok(`${real}\n`);
      if (!isClaude(program)) return err(`${program}: not found`, 127);
      const sub = args
        .filter((a) => !a.startsWith('-'))
        .slice(0, 3)
        .join(' ');
      for (const failing of fake.fail) if (sub.startsWith(failing)) return err(`${failing} failed`);
      if (sub === 'plugin list') {
        if (fake.garbledList) return ok('Installed plugins:\n  iii@iii\n');
        return ok(
          JSON.stringify(
            fake.installs.map((i) => ({
              id: i.id,
              version: '0.3.0',
              scope: i.scope,
              enabled: i.enabled,
              installPath: '/home/u/.claude/plugins/cache/iii/iii/0.3.0',
              ...(i.projectPath ? { projectPath: i.projectPath } : {}),
            })),
          ),
        );
      }
      if (sub === 'plugin marketplace list') {
        return ok(
          JSON.stringify(
            fake.marketplaces.map((name) => ({ name, source: 'github', repo: `x/${name}` })),
          ),
        );
      }
      if (sub.startsWith('plugin marketplace add')) {
        if (args.includes('iii-hq/agent-plugins-claude')) fake.marketplaces.push('iii');
        return ok(
          JSON.stringify({ command: 'marketplace-add', outcome: 'ok', marketplace: 'iii' }),
        );
      }
      if (sub.startsWith('plugin install')) {
        const id = args[args.indexOf('install') + 1];
        if (!fake.marketplaces.includes(id.split('@')[1]))
          return err(`Plugin "${id}" not found in any configured marketplace`);
        const scope = args[args.indexOf('--scope') + 1] as Install['scope'];
        fake.installs.push({ id, scope, enabled: true, projectPath: cwd });
        return ok(JSON.stringify({ command: 'install', outcome: 'ok', pluginId: id, scope }));
      }
      if (sub.startsWith('plugin enable')) {
        const id = args[args.indexOf('enable') + 1];
        const entry = fake.installs.find((i) => i.id === id && i.projectPath === cwd);
        if (entry) entry.enabled = true;
        return ok(JSON.stringify({ command: 'enable', outcome: 'ok' }));
      }
      return err(`unknown command ${sub}`, 2);
    },
  };
  return fake;
}
