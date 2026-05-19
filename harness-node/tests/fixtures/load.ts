import { readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));

export type Fixture = {
  session_id: string;
  entries: Array<{
    id: string;
    parent_id: string | null;
    timestamp: number;
    type: 'message';
    message: unknown;
  }>;
};

export function loadFixture(name: string): Fixture {
  return JSON.parse(readFileSync(join(here, 'sessions', `${name}.json`), 'utf8'));
}
