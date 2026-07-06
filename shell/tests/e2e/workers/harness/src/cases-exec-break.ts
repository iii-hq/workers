import { expect, expectEqual, type CaseContext, type TestCase } from './cases.ts';

export const EXEC_BREAK_CASES: TestCase[] = [
  {
    name: 'exec_empty_command_string_rejected',
    async run({ call, expectError }) {
      await expectError(
        () => call('shell::exec', { command: '' }),
        'empty command',
      );
    },
  },
  {
    // Renamed from `_reports_as_missing` — that name encoded a legacy bug
    // where the loose Value handler conflated wrong-type with absent. The
    // typed schema (ExecRequest) now produces a field-aware error so the
    // LLM can distinguish the two cases.
    name: 'exec_command_number_reports_wrong_type',
    async run({ call, expectError }) {
      await expectError(
        () => call('shell::exec', { command: 42 }),
        /'command'.+string/,
      );
    },
  },
  {
    name: 'exec_command_boolean_reports_wrong_type',
    async run({ call, expectError }) {
      await expectError(
        () => call('shell::exec', { command: true }),
        /'command'.+string/,
      );
    },
  },
  {
    name: 'exec_args_field_not_array_rejected',
    async run({ call, expectError }) {
      await expectError(
        () => call('shell::exec', { command: 'echo', args: 'not-an-array' as unknown as string[] }),
        'must be an array of strings',
      );
    },
  },
  {
    name: 'exec_args_null_entry_rejected',
    async run({ call, expectError }) {
      await expectError(
        () => call('shell::exec', { command: 'echo', args: [null] as unknown as string[] }),
        'must be a string',
      );
    },
  },
  {
    name: 'exec_args_object_entry_rejected',
    async run({ call, expectError }) {
      await expectError(
        () => call('shell::exec', { command: 'echo', args: [{}] as unknown as string[] }),
        'must be a string',
      );
    },
  },
  {
    name: 'exec_args_bool_entry_rejected',
    async run({ call, expectError }) {
      await expectError(
        () => call('shell::exec', { command: 'echo', args: [true] as unknown as string[] }),
        'must be a string',
      );
    },
  },
  {
    name: 'exec_command_with_embedded_nul_rejected',
    async run({ call }) {
      let threw = false;
      try {
        await call('shell::exec', { command: 'echo\0evil' });
      } catch {
        threw = true;
      }
      expect(threw, 'embedded NUL in command must reject');
    },
  },
  {
    name: 'exec_arg_with_embedded_nul_no_crash',
    async run({ call }) {
      let threw = false;
      let exitOk = false;
      try {
        const r = await call('shell::exec', {
          command: 'echo',
          args: ['safe', 'has\0nul'],
        });
        exitOk = typeof r.exit_code === 'number';
      } catch {
        threw = true;
      }
      expect(threw || exitOk, 'either rejects cleanly or returns a result');
    },
  },
  {
    name: 'exec_timeout_ms_zero_triggers_immediate_timeout',
    async run({ call }) {
      const r = await call('shell::exec', {
        command: 'sleep',
        args: ['1'],
        timeout_ms: 0,
      });
      expectEqual(r.timed_out, true, 'timeout_ms=0 should produce timed_out=true');
      expectEqual(r.exit_code, null, 'exit_code is null on timeout');
    },
  },
  {
    name: 'exec_timeout_ms_negative_silently_uses_default',
    async run({ call }) {
      const r = await call('shell::exec', {
        command: 'echo',
        args: ['ok'],
        timeout_ms: -1 as unknown as number,
      });
      expectEqual(r.exit_code, 0, 'echo still runs (timeout_ms=-1 ignored)');
      expectEqual(r.timed_out, false, 'fallback to default_timeout_ms');
    },
  },
  {
    name: 'exec_timeout_ms_float_silently_uses_default',
    async run({ call }) {
      const r = await call('shell::exec', {
        command: 'echo',
        args: ['ok'],
        timeout_ms: 1.5 as unknown as number,
      });
      expectEqual(r.exit_code, 0, 'echo still runs (timeout_ms=1.5 ignored)');
      expectEqual(r.timed_out, false, 'fallback to default_timeout_ms');
    },
  },
  {
    name: 'exec_timeout_ms_max_safe_int_clamps',
    async run({ call }) {
      const r = await call('shell::exec', {
        command: 'echo',
        args: ['ok'],
        timeout_ms: Number.MAX_SAFE_INTEGER,
      });
      expectEqual(r.exit_code, 0, 'completes well before clamped cap');
      expect(r.duration_ms < 5000, `duration_ms < 5000 (got ${r.duration_ms})`);
      expectEqual(r.timed_out, false, 'not timed out');
    },
  },
  {
    name: 'exec_argv_flood_1000_args_no_crash',
    async run(ctx: CaseContext) {
      const args = Array.from({ length: 1000 }, () => 'a');
      let threw = false;
      let exit: number | null = null;
      try {
        const r = await ctx.call('shell::exec', { command: 'echo', args });
        exit = r.exit_code;
      } catch {
        threw = true;
      }
      expect(threw || exit === 0, 'either rejected cleanly or echoed all args');
      const r2 = await ctx.call('shell::exec', { command: 'echo', args: ['after'] });
      expectEqual(r2.exit_code, 0, 'worker survives argv flood');
    },
  },
  {
    name: 'exec_denylisted_command_with_extra_fields_still_rejected',
    async run({ call, expectError }) {
      await expectError(
        () => call('shell::exec', {
          command: 'echo',
          args: ['harness_denylist_marker'],
          unknown_field: 'ignored',
          another_extra: 42,
        }),
        'denylist',
      );
    },
  },
  {
    name: 'exec_extra_unknown_fields_on_happy_path_tolerated',
    async run({ call }) {
      const r = await call('shell::exec', {
        command: 'echo',
        args: ['hi'],
        unknown_field: 'should be ignored',
        nested: { also: 'ignored' },
      });
      expectEqual(r.exit_code, 0, 'unknown fields tolerated');
      expectEqual(r.stdout, 'hi\n', 'stdout intact');
    },
  },
];
