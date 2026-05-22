import { registerWorker } from 'iii-sdk'

const iii = registerWorker(process.env.III_URL ?? 'ws://127.0.0.1:49134')

await iii.trigger({
  function_id: 'database::execute',
  payload: {
    db: 'primary',
    sql: 'CREATE TABLE IF NOT EXISTS users (id INTEGER PRIMARY KEY, email TEXT)',
  },
})

await iii.trigger({
  function_id: 'database::execute',
  payload: {
    db: 'primary',
    sql: 'INSERT INTO users (email) VALUES (?), (?)',
    params: ['a@x', 'b@x'],
  },
})

const { rows } = await iii.trigger({
  function_id: 'database::query',
  payload: {
    db: 'primary',
    sql: 'SELECT id, email FROM users ORDER BY id',
  },
})
