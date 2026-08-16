import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const here = path.dirname(fileURLToPath(import.meta.url));
const schemasDir = [
  path.join(here, 'schemas'),
  path.join(here, '../../schemas'),
  path.join(process.cwd(), 'schemas'),
].find((dir) => fs.existsSync(dir));

if (!schemasDir) {
  throw new Error(`opengantry: schemas/ not found next to ${here} or cwd ${process.cwd()}`);
}

export function loadSchema(fileName) {
  return JSON.parse(fs.readFileSync(path.join(schemasDir, fileName), 'utf8'));
}
