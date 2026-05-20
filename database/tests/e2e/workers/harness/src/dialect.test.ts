// Run with: node --test --import tsx ./src/dialect.test.ts
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { dialects } from './dialect.ts';

test('sqlite uses ? placeholder and AUTOINCREMENT id', () => {
  assert.equal(dialects.sqlite_db.placeholder(1), '?');
  assert.equal(dialects.sqlite_db.placeholder(7), '?');
  assert.equal(dialects.sqlite_db.idColumnDDL(), 'INTEGER PRIMARY KEY AUTOINCREMENT');
});

test('postgres uses $N placeholders and BIGSERIAL id', () => {
  assert.equal(dialects.pg_db.placeholder(1), '$1');
  assert.equal(dialects.pg_db.placeholder(2), '$2');
  assert.equal(dialects.pg_db.idColumnDDL(), 'BIGSERIAL PRIMARY KEY');
});

test('mysql uses ? placeholder and AUTO_INCREMENT BIGINT id', () => {
  assert.equal(dialects.mysql_db.placeholder(1), '?');
  assert.equal(dialects.mysql_db.placeholder(3), '?');
  assert.equal(dialects.mysql_db.idColumnDDL(), 'BIGINT AUTO_INCREMENT PRIMARY KEY');
});

test('exposes exactly three driver keys', () => {
  assert.deepEqual(Object.keys(dialects).sort(), ['mysql_db', 'pg_db', 'sqlite_db']);
});
