import { registerWorker } from 'iii-sdk'

const url = process.env.III_URL
if (!url || url.includes(':49134')) throw new Error('the fixture needs the run engine III_URL')
const iii = registerWorker(url, { workerName: 'api' })
iii.registerFunction('fixture::health', async () => ({ status_code: 200, body: { ok: true } }))
iii.registerFunction('fixture::hello', async () => ({ status_code: 200, body: { hello: 'world' } }))
iii.observe({ function_id: 'http::on-request', args: { method: 'GET', path: '/api/health' } }, 'fixture::health')
iii.observe({ function_id: 'http::on-request', args: { method: 'GET', path: '/api/hello' } }, 'fixture::hello')
