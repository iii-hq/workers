import type { TestCase } from './cases.ts';
import { expect, expectEqual } from './cases.ts';

/**
 * Boundary-value cases targeting type encoding, NULL handling, and string
 * round-trip. Each test creates and drops its own scratch table so it stays
 * independent of the shared `t` / `outbox` tables touched by the function suite.
 *
 * i64 boundary tests use inline SQL literals because JSON cannot carry an
 * exact i64 across all values (JS Number tops out at 2^53-1). The bug surface
 * the recent debugging found was on the *read-back* path (RowValue::BigInt →
 * JSON string in value.rs:90), which inline-literal inserts exercise just as
 * well as parameterized inserts. We test the param-decode path separately
 * within Number.MAX_SAFE_INTEGER.
 */
export const BOUNDARY_CASES: TestCase[] = [
  {
    name: 'i64 max round-trip (BIGINT-as-string)',
    // Gated to pg_db: postgres has a BIGINT/INT8 column type the driver can map
    // to RowValue::BigInt → JSON string for precision. SQLite has no column-type
    // distinction (single INTEGER affinity, value-dependent storage), so its
    // driver maps to RowValue::Int unconditionally → JSON Number → precision
    // loss above 2^53. MySQL's BIGINT-mapping behavior is a separate finding;
    // see mysql_db: i64 max round-trip failure for whether the mysql driver
    // also drops to Int despite having distinct BIGINT type info available.
    applies: ['pg_db'],
    async run({ driver, call }) {
      await call('iii-database::execute', { db: driver, sql: 'DROP TABLE IF EXISTS bx_i64max' });
      await call('iii-database::execute', { db: driver, sql: 'CREATE TABLE bx_i64max (n BIGINT NOT NULL)' });
      await call('iii-database::execute', {
        db: driver,
        sql: 'INSERT INTO bx_i64max (n) VALUES (9223372036854775807)',
      });
      const q = await call('iii-database::query', { db: driver, sql: 'SELECT n FROM bx_i64max' });
      const v = q.rows[0].n;
      expectEqual(v, '9223372036854775807', 'i64::MAX preserved as JSON string');
      await call('iii-database::execute', { db: driver, sql: 'DROP TABLE bx_i64max' });
    },
  },
  {
    name: 'i64 min round-trip (BIGINT-as-string)',
    applies: ['pg_db'],
    async run({ driver, call }) {
      await call('iii-database::execute', { db: driver, sql: 'DROP TABLE IF EXISTS bx_i64min' });
      await call('iii-database::execute', { db: driver, sql: 'CREATE TABLE bx_i64min (n BIGINT NOT NULL)' });
      await call('iii-database::execute', {
        db: driver,
        sql: 'INSERT INTO bx_i64min (n) VALUES (-9223372036854775808)',
      });
      const q = await call('iii-database::query', { db: driver, sql: 'SELECT n FROM bx_i64min' });
      expectEqual(q.rows[0].n, '-9223372036854775808', 'i64::MIN preserved as JSON string');
      await call('iii-database::execute', { db: driver, sql: 'DROP TABLE bx_i64min' });
    },
  },
  {
    name: 'large integer precision documentation (sqlite + mysql)',
    // SQLite/MySQL drivers currently emit large i64 as JSON Number, not string.
    // Number.MAX_SAFE_INTEGER = 2^53 - 1 = 9007199254740991. We assert that
    // values within that bound round-trip exactly, documenting the working
    // contract while the BIGINT-as-string-test (pg-only) holds the bar above.
    applies: ['sqlite_db', 'mysql_db'],
    async run({ driver, call }) {
      await call('iii-database::execute', { db: driver, sql: 'DROP TABLE IF EXISTS bx_i64safe' });
      await call('iii-database::execute', { db: driver, sql: 'CREATE TABLE bx_i64safe (n BIGINT NOT NULL)' });
      // 9007199254740991 = Number.MAX_SAFE_INTEGER
      await call('iii-database::execute', {
        db: driver,
        sql: 'INSERT INTO bx_i64safe (n) VALUES (9007199254740991)',
      });
      const q = await call('iii-database::query', { db: driver, sql: 'SELECT n FROM bx_i64safe' });
      const v = q.rows[0].n;
      expect(
        v === 9007199254740991 || v === '9007199254740991',
        `MAX_SAFE_INTEGER round-trip: got ${JSON.stringify(v)}`,
      );
      await call('iii-database::execute', { db: driver, sql: 'DROP TABLE bx_i64safe' });
    },
  },
  {
    name: 'param-decode int into INT4 column (regression)',
    async run({ driver, dialect, call }) {
      // Recent debugging found a bug where the postgres driver wrote 8-byte i64 into
      // a 4-byte INT4 column, surfacing as `22P03 invalid_binary_representation`.
      // This case binds an i64-shaped JSON number (within Number.MAX_SAFE_INTEGER)
      // to a 32-bit-wide column type. Drivers must dispatch on column type width.
      const ph1 = dialect.placeholder(1);
      await call('iii-database::execute', { db: driver, sql: 'DROP TABLE IF EXISTS bx_int4' });
      // Use INT (postgres maps to INT4, mysql to INT, sqlite stores as INTEGER affinity).
      await call('iii-database::execute', { db: driver, sql: 'CREATE TABLE bx_int4 (n INT NOT NULL)' });
      await call('iii-database::execute', {
        db: driver,
        sql: `INSERT INTO bx_int4 (n) VALUES (${ph1})`,
        params: [12345],
      });
      const q = await call('iii-database::query', { db: driver, sql: 'SELECT n FROM bx_int4' });
      expectEqual(Number(q.rows[0].n), 12345, 'INT column round-trip');
      await call('iii-database::execute', { db: driver, sql: 'DROP TABLE bx_int4' });
    },
  },
  {
    name: 'NULL param insert and select',
    async run({ driver, dialect, call }) {
      const ph1 = dialect.placeholder(1);
      const ph2 = dialect.placeholder(2);
      await call('iii-database::execute', { db: driver, sql: 'DROP TABLE IF EXISTS bx_null' });
      await call('iii-database::execute', { db: driver, sql: 'CREATE TABLE bx_null (a INT NULL, b TEXT NULL)' });
      const r = await call('iii-database::execute', {
        db: driver,
        sql: `INSERT INTO bx_null (a, b) VALUES (${ph1}, ${ph2})`,
        params: [null, null],
      });
      expectEqual(r.affected_rows, 1, 'insert with null params');
      const q = await call('iii-database::query', {
        db: driver,
        sql: 'SELECT a, b FROM bx_null WHERE a IS NULL AND b IS NULL',
      });
      expectEqual(q.row_count, 1, 'one matching row with both nulls');
      expectEqual(q.rows[0].a, null, 'a is JSON null');
      expectEqual(q.rows[0].b, null, 'b is JSON null');
      await call('iii-database::execute', { db: driver, sql: 'DROP TABLE bx_null' });
    },
  },
  {
    name: 'empty string vs NULL distinction',
    async run({ driver, dialect, call }) {
      const ph1 = dialect.placeholder(1);
      const ph2 = dialect.placeholder(2);
      await call('iii-database::execute', { db: driver, sql: 'DROP TABLE IF EXISTS bx_emptynull' });
      // postgres/mysql/sqlite all distinguish '' from NULL; assert the worker doesn't conflate.
      await call('iii-database::execute', {
        db: driver,
        sql: `CREATE TABLE bx_emptynull (id ${dialect.idColumnDDL()}, s TEXT NULL)`,
      });
      await call('iii-database::execute', {
        db: driver,
        sql: `INSERT INTO bx_emptynull (s) VALUES (${ph1}), (${ph2})`,
        params: ['', null],
      });
      const q = await call('iii-database::query', {
        db: driver,
        sql: 'SELECT s FROM bx_emptynull ORDER BY id',
      });
      expectEqual(q.rows[0].s, '', 'first row is empty string, not null');
      expectEqual(q.rows[1].s, null, 'second row is null, not empty string');
      await call('iii-database::execute', { db: driver, sql: 'DROP TABLE bx_emptynull' });
    },
  },
  {
    name: 'UTF-8 round-trip (emoji + RTL + combining marks)',
    async run({ driver, dialect, call }) {
      const ph1 = dialect.placeholder(1);
      await call('iii-database::execute', { db: driver, sql: 'DROP TABLE IF EXISTS bx_utf8' });
      await call('iii-database::execute', { db: driver, sql: 'CREATE TABLE bx_utf8 (s TEXT NOT NULL)' });
      // Mix: emoji (4-byte UTF-8), RTL Arabic, Latin with combining acute, ZWSP, Han ideograph.
      const payload = '🔥مرحبا é​汉';
      await call('iii-database::execute', {
        db: driver,
        sql: `INSERT INTO bx_utf8 (s) VALUES (${ph1})`,
        params: [payload],
      });
      const q = await call('iii-database::query', { db: driver, sql: 'SELECT s FROM bx_utf8' });
      expectEqual(q.rows[0].s, payload, 'utf-8 round-trip exact equality');
      await call('iii-database::execute', { db: driver, sql: 'DROP TABLE bx_utf8' });
    },
  },
  {
    name: 'long string round-trip (64KB)',
    async run({ driver, dialect, call }) {
      const ph1 = dialect.placeholder(1);
      await call('iii-database::execute', { db: driver, sql: 'DROP TABLE IF EXISTS bx_long' });
      // MySQL TEXT caps at 64KB; LONGTEXT is unbounded. Use LONGTEXT on mysql to stay clear of headers.
      const colType = driver === 'mysql_db' ? 'LONGTEXT' : 'TEXT';
      await call('iii-database::execute', {
        db: driver,
        sql: `CREATE TABLE bx_long (s ${colType} NOT NULL)`,
      });
      const payload = 'x'.repeat(64 * 1024 - 16);
      await call('iii-database::execute', {
        db: driver,
        sql: `INSERT INTO bx_long (s) VALUES (${ph1})`,
        params: [payload],
      });
      const q = await call('iii-database::query', { db: driver, sql: 'SELECT s FROM bx_long' });
      expectEqual(
        (q.rows[0].s as string).length,
        payload.length,
        '64KB string length preserved',
      );
      await call('iii-database::execute', { db: driver, sql: 'DROP TABLE bx_long' });
    },
  },
  {
    name: 'float values including small subnormal',
    async run({ driver, dialect, call }) {
      const ph1 = dialect.placeholder(1);
      const ph2 = dialect.placeholder(2);
      const ph3 = dialect.placeholder(3);
      await call('iii-database::execute', { db: driver, sql: 'DROP TABLE IF EXISTS bx_float' });
      await call('iii-database::execute', {
        db: driver,
        sql: `CREATE TABLE bx_float (id ${dialect.idColumnDDL()}, f DOUBLE PRECISION NOT NULL)`,
      });
      await call('iii-database::execute', {
        db: driver,
        sql: `INSERT INTO bx_float (f) VALUES (${ph1}), (${ph2}), (${ph3})`,
        params: [0.0, 2.5, 1.5e-300],
      });
      const q = await call('iii-database::query', { db: driver, sql: 'SELECT f FROM bx_float ORDER BY id' });
      const fs = q.rows.map((r: any) => Number(r.f));
      expect(Math.abs(fs[0] - 0.0) < 1e-12, `f0 ≈ 0.0, got ${fs[0]}`);
      expect(Math.abs(fs[1] - 2.5) < 1e-12, `f1 ≈ 2.5, got ${fs[1]}`);
      expect(fs[2] < 1e-200 && fs[2] > 0, `f2 is small positive double, got ${fs[2]}`);
      await call('iii-database::execute', { db: driver, sql: 'DROP TABLE bx_float' });
    },
  },
  {
    name: 'JSONB column round-trip (object + array + nested null)',
    // Postgres-only: SQLite has no native JSON column type and MySQL's JSON
    // column is supported but the driver maps it through MyValue::Bytes →
    // RowValue::Text rather than RowValue::Json (different code path; not the
    // one this test targets). Postgres jsonb is decoded as RowValue::Json and
    // returned via `into_json` — the move-vs-clone path fixed in [H5].
    //
    // Scope is limited to JSON-shaped values (objects + arrays). The worker's
    // `JsonParam::from_json` only routes Object/Array variants to
    // `JsonParam::Json`; bare strings go through `JsonParam::Text` and would
    // bind as TEXT, not JSONB — that's a different code path.
    applies: ['pg_db'],
    async run({ driver, dialect, call }) {
      const ph1 = dialect.placeholder(1);
      await call('iii-database::execute', { db: driver, sql: 'DROP TABLE IF EXISTS bx_jsonb' });
      await call('iii-database::execute', {
        db: driver,
        sql: `CREATE TABLE bx_jsonb (id ${dialect.idColumnDDL()}, body JSONB NOT NULL, label TEXT NOT NULL)`,
      });
      // Each shape must round-trip exactly through the
      // RowValue::Json → into_json path without re-serialization quirks.
      const cases: Array<{ label: string; body: unknown }> = [
        { label: 'obj', body: { user: { id: 7, name: 'O\'Brien', tags: ['a', 'b'] }, count: 42 } },
        { label: 'arr', body: [1, 'two', null, true, { k: 'v' }] },
        { label: 'with_null', body: { a: null, b: 0 } },
        { label: 'empty_obj', body: {} },
        { label: 'empty_arr', body: [] },
      ];
      for (const c of cases) {
        const r = await call('iii-database::execute', {
          db: driver,
          sql: `INSERT INTO bx_jsonb (body, label) VALUES (${ph1}, '${c.label}')`,
          params: [c.body],
        });
        expectEqual(r.affected_rows, 1, `inserted ${c.label}`);
      }

      const q = await call('iii-database::query', {
        db: driver,
        sql: 'SELECT label, body FROM bx_jsonb ORDER BY id',
      });
      expectEqual(q.row_count, cases.length, 'all jsonb rows returned');
      // Postgres jsonb canonicalizes both whitespace AND object key order
      // (alphabetical by key). Compare semantic equality with a stable-key
      // canonicalization on both sides.
      const canon = (v: unknown): unknown => {
        if (Array.isArray(v)) return v.map(canon);
        if (v !== null && typeof v === 'object') {
          const out: Record<string, unknown> = {};
          for (const k of Object.keys(v as Record<string, unknown>).sort()) {
            out[k] = canon((v as Record<string, unknown>)[k]);
          }
          return out;
        }
        return v;
      };
      for (let i = 0; i < cases.length; i++) {
        expectEqual(q.rows[i].label, cases[i].label, `row ${i} label`);
        expectEqual(canon(q.rows[i].body), canon(cases[i].body), `row ${i} body (${cases[i].label}) round-trip`);
      }
      await call('iii-database::execute', { db: driver, sql: 'DROP TABLE bx_jsonb' });
    },
  },
  {
    name: 'special characters in string params (parameterized binding)',
    async run({ driver, dialect, call }) {
      const ph1 = dialect.placeholder(1);
      await call('iii-database::execute', { db: driver, sql: 'DROP TABLE IF EXISTS bx_special' });
      await call('iii-database::execute', { db: driver, sql: 'CREATE TABLE bx_special (s TEXT NOT NULL)' });
      // If the worker were string-interpolating, single-quote would terminate the literal
      // and the trailing "; DROP TABLE …" would execute. Proper parameter binding makes
      // the value inert — round-trip equality + table-still-exists asserts that.
      const payload = `O'Brien "the\\quoted"\n\t\r-- ; DROP TABLE bx_special`;
      await call('iii-database::execute', {
        db: driver,
        sql: `INSERT INTO bx_special (s) VALUES (${ph1})`,
        params: [payload],
      });
      const q = await call('iii-database::query', { db: driver, sql: 'SELECT s FROM bx_special' });
      expectEqual(q.rows[0].s, payload, 'special-char string round-trip');
      const q2 = await call('iii-database::query', {
        db: driver,
        sql: 'SELECT COUNT(*) AS c FROM bx_special',
      });
      expectEqual(Number(q2.rows[0].c), 1, 'table not dropped by injection-shaped payload');
      await call('iii-database::execute', { db: driver, sql: 'DROP TABLE bx_special' });
    },
  },
];
