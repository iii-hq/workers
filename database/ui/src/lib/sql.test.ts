import { describe, expect, it } from 'vitest'
import { splitStatements, tokenizeSql } from './sql'

const kinds = (text: string, dialect?: Parameters<typeof tokenizeSql>[1]) =>
  tokenizeSql(text, dialect)
    .filter((t) => t.kind !== 'space')
    .map((t) => `${t.kind}:${t.text}`)

describe('tokenizeSql', () => {
  it('colours keywords, identifiers, strings, numbers and operators', () => {
    expect(kinds("status = 'open' AND total >= 10.5")).toEqual([
      'ident:status',
      'operator:=',
      "string:'open'",
      'keyword:AND',
      'ident:total',
      'operator:>=',
      'number:10.5',
    ])
  })

  it('covers every character exactly once', () => {
    const text = `select "a""b", \`c\`, $1, :name, E'x\\'y' -- tail\n/* block */ from t;`
    const tokens = tokenizeSql(text, 'postgres')
    expect(tokens.map((t) => t.text).join('')).toBe(text)
    for (let i = 1; i < tokens.length; i++) expect(tokens[i].start).toBe(tokens[i - 1].end)
  })

  it('keeps doubled quotes inside a string and quoted identifiers whole', () => {
    expect(kinds(`'it''s' "my ""col"" " \`tick\``)).toEqual([`string:'it''s'`, `quoted:"my ""col"" "`, 'quoted:`tick`'])
  })

  it('follows the dialect where token boundaries differ', () => {
    expect(kinds('a # note', 'mysql')).toEqual(['ident:a', 'comment:# note'])
    expect(kinds('"text"', 'mysql')).toEqual(['string:"text"'])
    expect(kinds("'a\\'b'", 'mysql')).toEqual(["string:'a\\'b'"])
    expect(kinds('$tag$ a; b $tag$', 'postgres')).toEqual(['string:$tag$ a; b $tag$'])
    expect(kinds('$1 + $2')).toEqual(['param:$1', 'operator:+', 'param:$2'])
    expect(kinds('a::int ->> b')).toEqual(['ident:a', 'operator:::', 'ident:int', 'operator:->>', 'ident:b'])
  })

  it('treats an unterminated string or comment as running to the end', () => {
    expect(kinds("x = 'open")).toEqual(['ident:x', 'operator:=', "string:'open"])
    expect(kinds('x /* never closed')).toEqual(['ident:x', 'comment:/* never closed'])
  })
})

describe('splitStatements', () => {
  it('splits on top-level semicolons and drops empty statements', () => {
    expect(splitStatements('select 1;\n\n select 2 ;;  -- only a comment\n;').map((s) => s.text)).toEqual([
      'select 1',
      'select 2',
    ])
  })

  it('does not split inside strings, identifiers, comments or dollar bodies', () => {
    const text = [
      "insert into t values ('a;b');",
      'select "x;y" from t; -- c; d',
      '/* e; f */ select 3;',
      'create function f() returns int as $$ select 1; $$ language sql;',
    ].join('\n')
    expect(splitStatements(text, 'postgres').map((s) => s.text)).toEqual([
      "insert into t values ('a;b')",
      'select "x;y" from t',
      '-- c; d\n/* e; f */ select 3',
      'create function f() returns int as $$ select 1; $$ language sql',
    ])
  })

  it('reports where each statement sits in the buffer', () => {
    const text = '  select 1;\nselect 2'
    const [a, b] = splitStatements(text)
    expect(text.slice(a.start, a.end)).toBe('select 1')
    expect(text.slice(b.start, b.end)).toBe('select 2')
  })

  it('keeps a single statement without a semicolon whole', () => {
    expect(splitStatements('select * from t where a = 1')).toEqual([
      { text: 'select * from t where a = 1', start: 0, end: 27 },
    ])
  })
})
