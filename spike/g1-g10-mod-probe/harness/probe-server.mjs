// DEV COCKPIT, servidor de la sonda G-pre (desechable). Hace de app para G10.
// - Escucha solo en 127.0.0.1, en un puerto aleatorio.
// - Tras --delay-ms escribe %APPDATA%\dev-cockpit-probe\endpoint.json ({port, token, pid}),
//   fuera de ~/.claude.
// - Exige Bearer, rechaza Origin y comprueba Host (como §6.2.1, simplificado).
// - Guarda un registro saneado (sin rutas ni token) en ../results/server-<etiqueta>.json.
// - Termina tras --timeout-ms y borra endpoint.json al salir.
// Uso: node probe-server.mjs --label cli-2.1.289 [--delay-ms 20000] [--timeout-ms 240000]
import http from 'node:http'
import { randomBytes, timingSafeEqual } from 'node:crypto'
import { mkdirSync, writeFileSync, rmSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const arg = (name, dflt) => {
  const i = process.argv.indexOf(`--${name}`)
  return i > 0 ? process.argv[i + 1] : dflt
}
const label = String(arg('label', 'unlabeled')).replace(/[^A-Za-z0-9._-]/g, '_')
const delayMs = Number(arg('delay-ms', 20000))
const timeoutMs = Number(arg('timeout-ms', 240000))
if (!process.env.APPDATA) { console.error('APPDATA no definida'); process.exit(2) }

const here = dirname(fileURLToPath(import.meta.url))
const resultsDir = join(here, '..', 'results')
const endpointDir = join(process.env.APPDATA, 'dev-cockpit-probe')
const endpointFile = join(endpointDir, 'endpoint.json')
const token = randomBytes(32).toString('hex')
const t0 = Date.now()
const log = { label, startedAt: t0, delayMs, endpointWrittenAt: null, requests: [] }

const okToken = h => {
  const want = Buffer.from(`Bearer ${token}`)
  const got = Buffer.from(String(h ?? ''))
  return got.length === want.length && timingSafeEqual(got, want)
}

const server = http.createServer((req, res) => {
  let body = ''
  req.on('data', c => { body += c; if (body.length > 65536) req.destroy() })
  req.on('end', () => {
    const entry = { atMs: Date.now() - t0, method: req.method, url: req.url, hasOrigin: !!req.headers.origin }
    let status = 200
    if (req.headers.origin) status = 403
    else if (req.headers.host !== `127.0.0.1:${server.address().port}`) status = 421
    else if (!okToken(req.headers.authorization)) status = 401
    else if (req.url === '/hello' && req.method === 'POST') {
      try { entry.helloProbe = JSON.parse(body).probe } catch { status = 422 }
    } else if (!(req.url === '/health' && req.method === 'GET')) status = 404
    entry.status = status
    log.requests.push(entry)
    flush()
    res.writeHead(status, { 'Content-Type': 'application/json' })
    res.end(JSON.stringify({ ok: status === 200 }))
  })
})

function flush() {
  mkdirSync(resultsDir, { recursive: true })
  writeFileSync(join(resultsDir, `server-${label}.json`), JSON.stringify(log, null, 2))
}

function cleanup() {
  try { rmSync(endpointFile, { force: true }) } catch {}
  flush()
}

server.listen(0, '127.0.0.1', () => {
  const { port } = server.address()
  console.log(`probe-server: escuchando en 127.0.0.1:${port}; endpoint.json en ${delayMs} ms; fin en ${timeoutMs} ms`)
  setTimeout(() => {
    mkdirSync(endpointDir, { recursive: true })
    writeFileSync(endpointFile, JSON.stringify({ port, token, pid: process.pid, schema: 1 }))
    log.endpointWrittenAt = Date.now() - t0
    flush()
    console.log('probe-server: endpoint.json escrito')
  }, delayMs)
  setTimeout(() => { cleanup(); server.close(); process.exit(0) }, timeoutMs)
})
for (const sig of ['SIGINT', 'SIGTERM']) process.on(sig, () => { cleanup(); process.exit(0) })
