import { call } from 'iii-sdk'

await call('iii-database::execute', {
  db: 'primary',
  sql: 'CREATE TABLE IF NOT EXISTS users (id INTEGER PRIMARY KEY, email TEXT)'
})

await call('iii-database::execute', {
  db: 'primary',
  sql: 'INSERT INTO users (email) VALUES (?), (?)',
  params: ['a@x', 'b@x']
})

const { rows } = await call('iii-database::query', {
  db: 'primary',
  sql: 'SELECT id, email FROM users ORDER BY id'
})