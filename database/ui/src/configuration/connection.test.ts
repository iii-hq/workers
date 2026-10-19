// biome-ignore-all lint/suspicious/noTemplateCurlyInString: these literals intentionally exercise ${VAR} configuration templates.
import { describe, expect, it } from 'vitest'
import {
  connectionDisplay,
  connectionDriver,
  connectionNameError,
  connectionProblems,
  connectionShape,
  connectionSummary,
  isAbsolutePath,
  isHostKeyPin,
  isPem,
  looksLikeKeyPath,
  newConnectionEntry,
  parseConnectionUrl,
  pemSummary,
  REDACTED,
  secretNameFor,
  secretNamesInUse,
  supportsStructuredConnections,
  switchDriver,
  testConnectionPayload,
  testOutcomeText,
  uniqueSecretName,
  urlEntryToFields,
  valueKind,
} from './connection'

const CERT = '-----BEGIN CERTIFICATE-----\nMIIBszCCAVmgAwIBAgIU\n-----END CERTIFICATE-----\n'

describe('secret names', () => {
  it('derives DATABASE_<CONNECTION>_<FIELD>, uppercased and restricted to [A-Za-z0-9_]', () => {
    expect(secretNameFor('shop', 'password')).toBe('DATABASE_SHOP_PASSWORD')
    expect(secretNameFor('shop-prod.eu', 'ssh.private_key')).toBe('DATABASE_SHOP_PROD_EU_SSH_PRIVATE_KEY')
    expect(secretNameFor('café', 'tls.client_key')).toBe('DATABASE_CAF__TLS_CLIENT_KEY')
    expect(secretNameFor('a b', 'ssh.passphrase')).toBe('DATABASE_A_B_SSH_PASSPHRASE')
    expect(secretNameFor('x', 'ssh.password')).toBe('DATABASE_X_SSH_PASSWORD')
  })

  it('stays within the secrets worker’s 128-character limit and keeps the field', () => {
    const name = secretNameFor('a'.repeat(300), 'ssh.private_key')
    expect(name.length).toBe(128)
    expect(name.endsWith('_SSH_PRIVATE_KEY')).toBe(true)
    expect(name).toMatch(/^[A-Za-z_][A-Za-z0-9_.-]{0,127}$/)
  })

  it('picks a suffix when another field already references the derived name', () => {
    expect(uniqueSecretName('DATABASE_A_B_PASSWORD', new Set())).toBe('DATABASE_A_B_PASSWORD')
    expect(uniqueSecretName('DATABASE_A_B_PASSWORD', new Set(['DATABASE_A_B_PASSWORD']))).toBe(
      'DATABASE_A_B_PASSWORD_2',
    )
    expect(
      uniqueSecretName('DATABASE_A_B_PASSWORD', new Set(['DATABASE_A_B_PASSWORD', 'DATABASE_A_B_PASSWORD_2'])),
    ).toBe('DATABASE_A_B_PASSWORD_3')
  })

  it('collects the names other fields reference, never the edited one', () => {
    const databases = {
      'a-b': { driver: 'postgres', password: 'secret://DATABASE_A_B_PASSWORD' },
      a_b: {
        driver: 'postgres',
        password: 'secret://OTHER',
        ssh: { host: 'h', private_key: 'secret://DATABASE_A_B_SSH_PRIVATE_KEY' },
        tls: { client_key: 'env://CLIENT_KEY' },
      },
      legacy: { url: 'postgres://db/app' },
      opaque: '${DB}',
    }
    expect(secretNamesInUse(databases)).toEqual(
      new Set(['DATABASE_A_B_PASSWORD', 'OTHER', 'DATABASE_A_B_SSH_PRIVATE_KEY']),
    )
    const others = secretNamesInUse(databases, { connection: 'a_b', field: 'password' })
    expect(others.has('OTHER')).toBe(false)
    // The colliding handle sees the first one's name and moves aside.
    expect(uniqueSecretName(secretNameFor('a_b', 'password'), others)).toBe('DATABASE_A_B_PASSWORD_2')
  })
})

describe('valueKind', () => {
  it('tells references, templates, the redaction sentinel and inline values apart', () => {
    expect(valueKind(undefined)).toBe('empty')
    expect(valueKind('   ')).toBe('empty')
    expect(valueKind('secret://DATABASE_SHOP_PASSWORD')).toBe('secret')
    expect(valueKind('SECRET://X')).toBe('secret')
    expect(valueKind('env://PGPASSWORD')).toBe('env')
    expect(valueKind('${PGPASSWORD}')).toBe('template')
    expect(valueKind(REDACTED)).toBe('redacted')
    expect(valueKind('hunter2')).toBe('literal')
    expect(valueKind('secret://not a name')).toBe('literal')
    expect(valueKind(42)).toBe('opaque')
  })
})

describe('url ↔ fields', () => {
  it('splits a network url, decoding credentials and dropping a default port', () => {
    expect(parseConnectionUrl('postgres://app:p%40ss%2Fword@db.example.com:5432/shop')).toEqual({
      ok: true,
      fields: { driver: 'postgres', host: 'db.example.com', user: 'app', password: 'p@ss/word', database: 'shop' },
    })
    expect(parseConnectionUrl('mysql://root@127.0.0.1:53306/shop')).toEqual({
      ok: true,
      fields: { driver: 'mysql', host: '127.0.0.1', port: 53306, user: 'root', database: 'shop' },
    })
    expect(parseConnectionUrl('postgresql://[::1]/app')).toEqual({
      ok: true,
      fields: { driver: 'postgres', host: '::1', database: 'app' },
    })
  })

  it('splits sqlite urls into the file path', () => {
    expect(parseConnectionUrl('sqlite:./data/app.db')).toEqual({
      ok: true,
      fields: { driver: 'sqlite', database: './data/app.db' },
    })
    expect(parseConnectionUrl('sqlite:///var/lib/app.db')).toEqual({
      ok: true,
      fields: { driver: 'sqlite', database: '/var/lib/app.db' },
    })
    expect(parseConnectionUrl('sqlite::memory:')).toEqual({ ok: true, fields: { driver: 'sqlite', database: ':memory:' } })
  })

  it('refuses what the fields could not hold without loss', () => {
    for (const url of [
      '',
      '${DATABASE_URL}',
      'postgres://db/app?sslmode=require',
      'sqlite:./app.db?mode=ro',
      'sqlite:file:app.db',
      'https://example.com/db',
      'postgres:///app',
      'postgres://db/app/extra',
    ]) {
      expect(parseConnectionUrl(url).ok, url).toBe(false)
    }
  })

  it('converts an entry in place, keeping every other block', () => {
    const entry = {
      url: 'postgres://app:pw@db:6543/shop',
      tls: { mode: 'verify-full' },
      pool: { max: 4 },
      capture: 'native',
    }
    const converted = urlEntryToFields(entry)
    expect(converted).toEqual({
      driver: 'postgres',
      host: 'db',
      port: 6543,
      user: 'app',
      password: 'pw',
      database: 'shop',
      tls: { mode: 'verify-full' },
      pool: { max: 4 },
      capture: 'native',
    })
    expect(Object.keys(converted ?? {})[0]).toBe('driver')
    expect(urlEntryToFields({ url: '${DATABASE_URL}' })).toBeNull()
  })

  it('reads the shape and the driver of either form', () => {
    expect(connectionShape({ url: 'mysql://db/app' })).toBe('url')
    expect(connectionShape({ driver: 'mysql', host: 'db' })).toBe('structured')
    expect(connectionDriver({ url: 'mysql://db/app' })).toBe('mysql')
    expect(connectionDriver({ driver: 'sqlite', database: 'a.db' })).toBe('sqlite')
    expect(connectionDriver({ driver: 'oracle' })).toBe('unknown')
  })
})

describe('driver changes', () => {
  it('starts a new connection with the picked driver', () => {
    expect(newConnectionEntry('postgres', 'shop')).toEqual({ driver: 'postgres', host: 'localhost', database: 'shop' })
    expect(newConnectionEntry('sqlite', 'shop')).toEqual({ driver: 'sqlite', database: './data/shop.db' })
  })

  it('sets network keys aside for sqlite and restores them on the way back', () => {
    const pg = {
      driver: 'postgres',
      host: 'db',
      port: 6543,
      user: 'app',
      password: 'secret://DATABASE_SHOP_PASSWORD',
      database: 'shop',
      ssh: { host: 'bastion' },
      display: { icon: 'server' },
    }
    const toSqlite = switchDriver(pg, 'sqlite')
    expect(toSqlite.entry).toEqual({ driver: 'sqlite', database: 'shop', display: { icon: 'server' } })
    const back = switchDriver(toSqlite.entry, 'postgres', toSqlite.stash)
    expect(back.entry).toEqual({ ...pg })
    expect(back.stash).toEqual({})
  })

  it('drops a port that was only the old driver’s default', () => {
    expect(switchDriver({ driver: 'postgres', host: 'db', port: 5432 }, 'mysql').entry).toEqual({
      driver: 'mysql',
      host: 'db',
    })
    expect(switchDriver({ driver: 'postgres', host: 'db', port: 7000 }, 'mysql').entry.port).toBe(7000)
    expect(switchDriver({ driver: 'sqlite', database: 'a.db' }, 'mysql').entry.host).toBe('localhost')
  })
})

describe('schema detection', () => {
  it('finds the structured shape in the worker’s JSON schema', () => {
    const structured = {
      definitions: {
        DatabaseConfig: { type: 'object', properties: { url: {}, driver: {}, host: {}, ssh: {}, tls: {} } },
      },
    }
    const legacy = { definitions: { DatabaseConfig: { type: 'object', properties: { url: {}, tls: {}, pool: {} } } } }
    expect(supportsStructuredConnections(structured)).toBe(true)
    expect(supportsStructuredConnections(legacy)).toBe(false)
    expect(supportsStructuredConnections(null)).toBe(true)
  })
})

describe('test connection payload', () => {
  it('sends the whole entry, and the name only to fill a redacted value', () => {
    const entry = { driver: 'postgres', host: 'db', password: 'secret://P' }
    expect(testConnectionPayload('shop', entry, true)).toEqual({ connection: entry, timeout_ms: 8000 })
    const redacted = { ...entry, password: REDACTED }
    expect(testConnectionPayload('shop', redacted, true)).toEqual({
      connection: redacted,
      name: 'shop',
      timeout_ms: 8000,
    })
  })

  it('falls back to {url, tls} for a worker without structured entries', () => {
    expect(testConnectionPayload('shop', { url: 'postgres://db/app', tls: { mode: 'disable' } }, false)).toEqual({
      url: 'postgres://db/app',
      tls: { mode: 'disable' },
      timeout_ms: 8000,
    })
  })

  it('reports where a probe stopped, and what it reached', () => {
    expect(testOutcomeText({ ok: true, driver: 'postgres', latency_ms: 12, server_version: 'PostgreSQL 17.2' })).toBe(
      'Connected · PostgreSQL 17.2 · 12 ms',
    )
    expect(testOutcomeText({ ok: false, stage: 'ssh', message: 'host key mismatch' })).toBe(
      'SSH tunnel failed · host key mismatch',
    )
    expect(testOutcomeText({ ok: false, stage: 'quantum', message: 'x' })).toBe('quantum failed · x')
    expect(testOutcomeText({ ok: false })).toBe('Connection failed')
  })
})

describe('display and summaries', () => {
  it('keeps only valid colours and string icons', () => {
    expect(connectionDisplay({ display: { icon: 'server', color: '#22C55E' } })).toEqual({
      icon: 'server',
      color: '#22C55E',
    })
    expect(connectionDisplay({ display: { icon: 7, color: 'green' } })).toEqual({})
    expect(connectionDisplay('${DB}')).toEqual({})
  })

  it('summarises where a structured entry points', () => {
    expect(connectionSummary({ driver: 'postgres', host: 'db', port: 6543, database: 'shop' })).toBe('db:6543/shop')
    expect(connectionSummary({ driver: 'mysql', host: 'db' })).toBe('db')
    expect(connectionSummary({ driver: 'sqlite', database: './a.db' })).toBe('./a.db')
    expect(connectionSummary({ url: 'postgres://db/app' })).toBe('')
  })

  it('recognises PEM material without printing it', () => {
    expect(isPem(CERT)).toBe(true)
    expect(isPem('/etc/ssl/ca.pem')).toBe(false)
    expect(pemSummary(CERT)).toBe('PEM · CERTIFICATE')
    expect(pemSummary(CERT + CERT)).toBe('2 PEM blocks · CERTIFICATE')
    expect(isPem('-----BEGIN OPENSSH PRIVATE KEY-----\nb3Bl\n-----END OPENSSH PRIVATE KEY-----')).toBe(true)
  })
})

describe('what the worker will refuse', () => {
  it('holds handles to the worker’s connection-name rule', () => {
    for (const ok of ['shop', 'shop-prod.eu', '_x', '9lives', 'a'.repeat(64)]) {
      expect(connectionNameError(ok), ok).toBeUndefined()
    }
    for (const bad of ['', 'my db', '-x', '.x', 'café', 'a'.repeat(65)]) {
      expect(connectionNameError(bad), bad).toBeDefined()
    }
  })

  it('recognises host-key pins, absolute paths and key files', () => {
    expect(isHostKeyPin(`SHA256:${'A'.repeat(43)}`)).toBe(true)
    expect(isHostKeyPin(`SHA256:${'A'.repeat(43)}=`)).toBe(true)
    expect(isHostKeyPin('SHA256:short')).toBe(false)
    expect(isHostKeyPin(`MD5:${'A'.repeat(43)}`)).toBe(false)
    expect(isAbsolutePath('/srv/q')).toBe(true)
    expect(isAbsolutePath('C:\\q')).toBe(true)
    expect(isAbsolutePath('queries')).toBe(false)
    expect(looksLikeKeyPath('/home/me/.ssh/id_ed25519')).toBe(true)
    expect(looksLikeKeyPath('~/.ssh/id_rsa')).toBe(true)
    expect(looksLikeKeyPath('-----BEGIN OPENSSH PRIVATE KEY-----\nx\n-----END OPENSSH PRIVATE KEY-----')).toBe(false)
    expect(looksLikeKeyPath('hunter2')).toBe(false)
  })

  it('names the fields an entry still needs', () => {
    expect(
      connectionProblems({
        driver: 'postgres',
        tls: { client_cert: '/c.crt' },
        host: 'db',
        ssh: { host: 'b', user: 'u', passphrase: 'secret://P', host_key: 'SHA256:nope' },
        folder: 'relative/queries',
      }).map((p) => p.field),
    ).toEqual(['user', 'tls', 'ssh', 'ssh.passphrase', 'ssh.host_key', 'folder'])
    const blank = { driver: 'mysql', host: ' ', ssh: { host: '', user: '', password: 'secret://P' } }
    expect(connectionProblems(blank).map((p) => p.field)).toEqual(['host', 'user', 'ssh.host', 'ssh.user'])
    expect(connectionProblems({ driver: 'sqlite' })).toEqual([
      { field: 'database', message: 'Required: the path of the database file.' },
    ])
    expect(connectionProblems({ url: 'postgres://db/app' })).toEqual([])
    expect(
      connectionProblems({
        driver: 'postgres',
        host: 'db',
        user: 'app',
        tls: { client_cert: '/c.crt', client_key: 'secret://K' },
        ssh: { host: 'b', user: 'u', private_key: 'secret://K2', host_key: `SHA256:${'b'.repeat(43)}` },
        folder: '/srv/q',
      }),
    ).toEqual([])
  })
})
