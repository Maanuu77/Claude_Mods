// DEV COCKPIT, sonda G-pre (desechable). SPEC §13.2, G1 y G10.
// Solo observa: ningún hook bloquea, reescribe ni llama al modelo.
// No transmite rutas, títulos ni contenido: solo booleanos, versiones,
// códigos de error saneados y tiempos.
//
// Reglas del analizador estático del motor (vistas en T1 con 2.1.289):
// - `$` solo se pasa a funciones declaradas al nivel superior del fichero;
// - `$` se escribe siempre como `$.nombre.método(...)` en el punto de llamada;
// - el valor de `on(...)` no se guarda, solo admite `.catch(...)`.
import type { Register } from 'claude-code'

const PROBE_DIR = 'dev-cockpit-probe'
const POLL_MS = 5_000 // la sonda vigila más a menudo que el mod real (15 s) para acortar la prueba
const GIVE_UP_MS = 180_000

const report: Record<string, unknown> = { probe: 'core', schema: 2, loadedAt: Date.now() }
const env: { appData?: string; localAppData?: string } = {}

export const register: Register = on => {
  on('session.start', async ($, e, next) => {
    const started = await next(e)
    void runProbe($)
    return started
  })

  on('command.run', { command: 'probe' }, async () => ({
    text: '```json\n' + JSON.stringify(report, null, 2) + '\n```',
  }))
}

// eslint-disable-next-line @typescript-eslint/no-explicit-any
async function runProbe($: any): Promise<void> {
  try {
    await $.command.register({ name: 'probe', description: 'DEV COCKPIT G-pre: muestra el informe de la sonda' })
    report.commandRegister = 'ok'
  } catch (err) {
    report.commandRegister = errText(err)
  }
  await checkG1($)
  await checkG10($)
}

// ---------- G1: cada API del diseño, llamada de verdad ----------
// eslint-disable-next-line @typescript-eslint/no-explicit-any
async function checkG1($: any): Promise<void> {
  const g1: Record<string, unknown> = {}
  report.g1 = g1
  try { const v = await $.session.version(); report.engine = v; g1['session.version'] = 'ok' } catch (err) { g1['session.version'] = errText(err) }
  try { await $.session.id(); g1['session.id'] = 'ok' } catch (err) { g1['session.id'] = errText(err) }
  try {
    const u = await $.session.usage()
    g1['session.usage'] = { ok: true, keys: Object.keys(u ?? {}), rateLimitKinds: (u?.rateLimits ?? []).map((r: { kind: string }) => r.kind), hasCost: u?.cost !== undefined }
  } catch (err) { g1['session.usage'] = errText(err) }
  try { const a = await $.agent.list(); g1['agent.list'] = { ok: true, count: Array.isArray(a) ? a.length : null } } catch (err) { g1['agent.list'] = errText(err) }
  try { g1['clock.now'] = typeof (await $.clock.now()) === 'number' ? 'ok' : 'unexpected' } catch (err) { g1['clock.now'] = errText(err) }
  try { const t = $.clock.after(60_000, () => undefined); t.cancel(); g1['clock.after'] = 'ok' } catch (err) { g1['clock.after'] = errText(err) }
  try { const t = $.clock.every(60_000, () => undefined); t.cancel(); g1['clock.every'] = 'ok' } catch (err) { g1['clock.every'] = errText(err) }
  try { env.appData = await $.env.get('APPDATA'); env.localAppData = await $.env.get('LOCALAPPDATA'); g1['env.get'] = env.appData ? 'ok' : 'APPDATA ausente' } catch (err) { g1['env.get'] = errText(err) }
  try { await $.fs.exists($.plugin.root); g1['fs.exists'] = 'ok' } catch (err) { g1['fs.exists'] = errText(err) }
  try { $.ui.status('dev-cockpit probe'); $.ui.status(undefined); g1['ui.status'] = 'ok' } catch (err) { g1['ui.status'] = errText(err) }
  // http.fetch, fs.stat, fs.read y fs.write se ejercitan en G10 y en persist()
  await persist($)
}

// ---------- G10: leer endpoint.json fuera de ~/.claude y conectar; reconexión S1 ----------
// eslint-disable-next-line @typescript-eslint/no-explicit-any
async function checkG10($: any): Promise<void> {
  const g10: Record<string, unknown> = { pollMs: POLL_MS }
  report.g10 = g10
  const first = await tryEndpoints($)
  g10.firstAttempt = first.summary
  if (first.connected) {
    g10.scenario = 'endpoint ya presente al arrancar'
    await persist($)
    return
  }
  g10.scenario = 'S1: endpoint ausente al arrancar; vigilancia hasta que aparezca'
  await persist($)
  const waitStart = Date.now()
  let ticks = 0
  let done = false
  const timer = $.clock.every(POLL_MS, () => {
    if (done) return
    ticks++
    void onTick($, waitStart, ticks).then(finished => {
      if (finished && !done) { done = true; timer.cancel() }
    })
  })
}

// eslint-disable-next-line @typescript-eslint/no-explicit-any
async function onTick($: any, waitStart: number, ticks: number): Promise<boolean> {
  const attempt = await tryEndpoints($)
  const waitedMs = Date.now() - waitStart
  if (!attempt.connected && waitedMs <= GIVE_UP_MS) return false
  const g10 = report.g10 as Record<string, unknown>
  g10.reconnect = { connected: attempt.connected, ticks, waitedMs, last: attempt.summary }
  await persist($)
  return true
}

// eslint-disable-next-line @typescript-eslint/no-explicit-any
async function tryEndpoints($: any): Promise<{ connected: boolean; summary: Record<string, unknown> }> {
  const summary: Record<string, unknown> = {}
  const candidates: Array<[string, string | undefined]> = [
    ['%APPDATA%', env.appData && `${env.appData}\\${PROBE_DIR}\\endpoint.json`],
    ['%LOCALAPPDATA% (alternativa A)', env.localAppData && `${env.localAppData}\\${PROBE_DIR}\\endpoint.json`],
  ]
  for (const [label, path] of candidates) {
    if (!path) { summary[label] = { skipped: 'variable de entorno ausente' }; continue }
    const entry: Record<string, unknown> = {}
    summary[label] = entry
    try { await $.fs.stat(path); entry.stat = 'ok' } catch (err) { entry.stat = sanitize(errText(err)) }
    let text: string
    try { text = await $.fs.read(path); entry.read = 'ok' } catch (err) { entry.read = sanitize(errText(err)); continue }
    let ep: { port?: number; token?: string }
    try { ep = JSON.parse(text) } catch { entry.parse = 'json inválido'; continue }
    const base = `http://127.0.0.1:${ep.port}`
    try { const r = await $.http.fetch(`${base}/health`, { headers: { Authorization: `Bearer ${ep.token}` } }); entry.health = r.status } catch (err) { entry.health = sanitize(errText(err)) }
    try { const r = await $.http.fetch(`${base}/health`); entry.healthWithoutToken = r.status } catch (err) { entry.healthWithoutToken = sanitize(errText(err)) }
    try {
      const r = await $.http.fetch(`${base}/hello`, {
        method: 'POST',
        headers: { Authorization: `Bearer ${ep.token}`, 'Content-Type': 'application/json' },
        body: JSON.stringify({ probe: 'core', t: Date.now() }),
      })
      entry.hello = r.status
      if (r.status === 200) return { connected: true, summary }
    } catch (err) { entry.hello = sanitize(errText(err)) }
  }
  return { connected: false, summary }
}

// eslint-disable-next-line @typescript-eslint/no-explicit-any
async function persist($: any): Promise<void> {
  report.updatedAt = Date.now()
  const ver = (report.engine as { version?: string } | undefined)?.version ?? 'unknown'
  // `$.plugin.root` es «el directorio que contiene plugin.json»: se normaliza a
  // la carpeta del mod y se sube un nivel, hasta spike/g1-g10-mod-probe/results.
  const base = String($.plugin.root).replace(/[\\/]\.claude-plugin[\\/]?$/, '')
  try {
    await $.fs.write(`${base}/../results/core-${ver}.json`, JSON.stringify(report, null, 2))
    report.fsWrite = 'ok'
  } catch (err) {
    report.fsWrite = sanitize(errText(err))
  }
}

function sanitize(s: string): string {
  let out = s
  if (env.appData) out = out.split(env.appData).join('%APPDATA%')
  if (env.localAppData) out = out.split(env.localAppData).join('%LOCALAPPDATA%')
  return out
}

function errText(err: unknown): string {
  return err instanceof Error ? `${err.name}: ${err.message}`.slice(0, 300) : String(err).slice(0, 300)
}
