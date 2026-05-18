import type { ISdk } from '../../runtime/iii.js';
import { listAllBudgets } from '../store.js';

export function register(iii: ISdk): void {
  iii.registerFunction(
    'budget::list',
    async (payload: unknown) => {
      const obj = (payload ?? {}) as Record<string, unknown>;
      const ws = typeof obj.workspace_id === 'string' ? obj.workspace_id : undefined;
      let all = await listAllBudgets(iii);
      if (ws) all = all.filter((b) => b.workspace_id === ws);
      all.sort((a, b) => b.created_at - a.created_at);
      return { budgets: all };
    },
    { description: 'List budgets, newest first.' },
  );
}
