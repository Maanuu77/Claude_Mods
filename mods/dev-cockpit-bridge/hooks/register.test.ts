// Tests del mod dentro del propio motor: `claude plugin test mods/dev-cockpit-bridge`.
//
// Los hooks que registra el test están debajo del mod y hacen de motor y de mundo
// exterior: el reloj, el entorno, el disco y la app (HTTP) se contestan desde aquí.
import { expect, mock, test } from 'claude-code/testing'
import type { On } from 'claude-code'

const APPDATA = 'C:\\Users\\dev\\AppData\\Roaming'
const ENDPOINT = `${APPDATA}\\dev-cockpit\\endpoint.json`
const CWD = 'C:\\work\\demo'
const TRAP = 'SECRETO-TRAMPA'

type Sent = { path: string; token: string; body: any } // eslint-disable-line @typescript-eslint/no-explicit-any

/** Una app de mentira y el mundo mínimo que necesita el mod. */
function world(on: On, options: { isAppOpen: boolean; engine?: string; isSupported?: boolean }) {
  const app = {
    isOpen: options.isAppOpen,
    token: 'token-1',
    ackSeq: 0,
    /** Estado HTTP que devuelve `/v1/events`; 200 por defecto. */
    eventsStatus: 200,
    sent: [] as Sent[],
    status: [] as Array<string | undefined>,
    events: () => app.sent.filter(s => s.path === '/v1/events').flatMap(s => s.body.events as any[]), // eslint-disable-line @typescript-eslint/no-explicit-any
    hellos: () => app.sent.filter(s => s.path === '/v1/hello'),
  }
  const clock = mock.clock(on)
  mock.env(on, { APPDATA })
  on('session.start', (_$, e) => ({ cwd: e.cwd }))
  on('session.id', () => ({ value: 'session-1' }))
  on('session.version', () => ({ value: { version: options.engine ?? '2.1.321' } }))
  on('session.usage', () => ({
    value: { startedAt: 0, context: { window: 200_000, tokens: 86_000, percent: 43 }, rateLimits: [], cost: { usd: 1.25 } },
  }))
  on('agent.list', () => ({ value: [] }))
  on('command.register', () => ({ value: { command: 'cockpit' } }))
  on('ui.status', (_$, e) => {
    app.status.push(e.text)
    return { value: undefined }
  })
  on('fs.stat', () => {
    if (!app.isOpen) throw new Error('ENOENT')
    return { value: { kind: 'file', size: 64, mtimeMs: 0 } as never }
  })
  on('fs.read', () => {
    if (!app.isOpen) throw new Error('ENOENT')
    return { value: JSON.stringify({ port: 4242, token: app.token, pid: 1, schema: 1 }) }
  })
  on('http.fetch', (_$, e) => {
    if (!app.isOpen) throw new Error('ECONNREFUSED')
    const path = e.url.replace('http://127.0.0.1:4242', '')
    const token = (e.init?.headers?.Authorization ?? '').replace('Bearer ', '')
    const reply = (status: number, body: unknown = {}) => ({
      value: { status, ok: status === 200, headers: {}, text: JSON.stringify(body) },
    })
    if (token !== app.token) return reply(401)
    const raw = e.init?.body
    const body = JSON.parse(typeof raw === 'string' ? raw : '{}')
    app.sent.push({ path, token, body })
    if (path === '/v1/hello') {
      return reply(200, {
        schema: 1,
        privacy: { policy_id: 'app-default-1', capture_task_titles: true, record_paths: true, capture_shell_program: false },
        events: { ack_seq: app.ackSeq },
        engine_supported: options.isSupported ?? true,
      })
    }
    if (app.eventsStatus !== 200) return reply(app.eventsStatus)
    for (const event of body.events) app.ackSeq = Math.max(app.ackSeq, event.seq)
    return reply(200, { ack_seq: app.ackSeq })
  })
  return { app, clock }
}

const START = { cwd: CWD, surface: 'terminal', isInteractive: true } as const

test('antes del handshake no sale ninguna ruta; después salen relativas', async ($, on) => {
  const { app, clock } = world(on, { isAppOpen: false })
  on('tool.call', () => ({ result: {} }))
  await $.session.start(START)
  await clock.settle()

  // La app está cerrada: esto se encola con la política más restrictiva.
  await $.tool.call({ tool: 'Edit', file_path: `${CWD}\\src\\antes.rs`, old_string: TRAP, new_string: TRAP })
  await $.tool.call({ tool: 'WebFetch', url: `https://docs.example.com/x?token=${TRAP}`, prompt: TRAP })
  await clock.advance(100)
  expect(app.sent).toEqual([])
  expect(app.status.at(-1)).toBe('○ cockpit')

  // La app se abre. El mod lo descubre en su siguiente comprobación (≤ 15 s).
  app.isOpen = true
  await clock.advance(15_000)

  expect(app.hellos().length).toBe(1)
  const hello = app.hellos()[0]!.body
  expect(hello.applied_policy.id).toBe('pre_handshake')
  expect(hello.reason).toBe('startup')
  expect(hello.session.id).toBe('session-1')
  const queued = app.events()
  expect(queued.map(e => e.kind)).toEqual(['tool_started', 'tool_finished', 'tool_started', 'tool_finished'])
  expect(queued[0].meta).toEqual({})
  expect(queued[2].meta).toEqual({})
  expect(app.status.at(-1)).toBe('◉ cockpit')

  // Con la política de la app ya adoptada, la ruta sale relativa y el host, solo.
  await $.tool.call({ tool: 'Edit', file_path: `${CWD}\\src\\despues.rs`, old_string: TRAP, new_string: TRAP, replace_all: true })
  await $.tool.call({ tool: 'Read', file_path: 'C:\\Users\\otro\\privado\\notas.md' })
  await $.tool.call({ tool: 'WebFetch', url: `https://Docs.Example.com:8443/x?token=${TRAP}`, prompt: TRAP })
  await $.tool.call({ tool: 'Bash', command: `curl -H "Authorization: ${TRAP}" https://example.com`, run_in_background: false })
  await $.tool.call({ tool: 'Grep', pattern: TRAP, path: `${CWD}\\src` })
  await clock.advance(100)

  const started = app.events().filter(e => e.kind === 'tool_started').slice(2)
  expect(started.map(e => [e.tool, e.meta])).toEqual([
    ['Edit', { rel_path: 'src/despues.rs', replace_all: true }],
    ['Read', { rel_path: '<externo>', ext: '.md' }],
    ['WebFetch', { host: 'docs.example.com' }],
    ['Bash', { run_in_background: false }],
    ['Grep', {}],
  ])

  // Nada de lo enviado, antes o después, lleva contenido ni rutas absolutas.
  const wire = JSON.stringify(app.sent)
  expect(wire.includes(TRAP)).toBe(false)
  expect(wire.includes('antes.rs')).toBe(false)
  expect(wire.includes('C:')).toBe(false)
  expect(wire.includes('privado')).toBe(false)
  expect(wire.includes('curl')).toBe(false)
})

test('la herramienta se anuncia antes de terminar y se cierra con su estado', async ($, on) => {
  const { app, clock } = world(on, { isAppOpen: true })
  on('tool.call', async (_$, e) => {
    await clock.sleep(5_000)
    return e.tool === 'Bash' ? { result: {}, isError: true as const } : { result: {} }
  })
  await $.session.start(START)
  await clock.settle()

  const slow = $.tool.call({ tool: 'Read', file_path: `${CWD}\\a.rs` })
  await clock.advance(100)
  expect(app.events().map(e => e.kind)).toEqual(['tool_started'])

  await clock.advance(5_000)
  await slow
  await clock.advance(100)
  expect(app.events().map(e => [e.kind, e.status])).toEqual([
    ['tool_started', undefined],
    ['tool_finished', 'ok'],
  ])

  const failing = $.tool.call({ tool: 'Bash', command: 'false' })
  await clock.advance(5_100)
  await failing
  await clock.advance(100)
  expect(app.events().at(-1)).toMatchObject({ kind: 'tool_finished', status: 'error' })

  // Las secuencias son estrictamente crecientes.
  const seqs = app.events().map(e => e.seq as number)
  expect(seqs).toEqual([...seqs].sort((a, b) => a - b))
  expect(new Set(seqs).size).toBe(seqs.length)
})

test('si la app se reinicia, el mod vuelve a leer el endpoint y repite el hello', async ($, on) => {
  const { app, clock } = world(on, { isAppOpen: true })
  on('tool.call', () => ({ result: {} }))
  await $.session.start(START)
  await clock.settle()
  expect(app.hellos().length).toBe(1)

  // Reinicio: token nuevo y la marca de agua de la app se conserva.
  app.token = 'token-2'
  await $.tool.call({ tool: 'Read', file_path: `${CWD}\\a.rs` })
  await clock.advance(100)

  expect(app.hellos().length).toBe(2)
  expect(app.hellos()[1]!.token).toBe('token-2')
  expect(app.hellos()[1]!.body.reason).toBe('reconnect')
  expect(app.hellos()[1]!.body.hello_seq).toBe(2)
  expect(app.events().map(e => e.kind)).toEqual(['tool_started', 'tool_finished'])
})

test('con la app cerrada la cola está acotada y la sesión no se entera', async ($, on) => {
  const { app, clock } = world(on, { isAppOpen: false })
  on('tool.call', () => ({ result: { ok: true } }))
  await $.session.start(START)
  await clock.settle()

  for (let i = 0; i < 1_100; i++) {
    const ran = await $.tool.call({ tool: 'Read', file_path: `${CWD}\\f${i}.rs` })
    expect(ran.deny).toBeUndefined()
  }
  app.isOpen = true
  await clock.advance(15_000)

  const hello = app.hellos()[0]!.body
  expect(hello.queue.pending).toBe(2_000)
  expect(hello.queue.dropped).toBe(200)
  // Se conserva lo más reciente y se envía en lotes.
  const events = app.events()
  expect(events.length).toBe(2_000)
  expect(events.at(-1).seq).toBe(2_200)
  expect(app.sent.filter(s => s.path === '/v1/events').every(s => s.body.events.length <= 500)).toBe(true)
})

test('un motor no soportado deja de enviar', async ($, on) => {
  const { app, clock } = world(on, { isAppOpen: true, engine: '2.1.285', isSupported: false })
  on('tool.call', () => ({ result: {} }))
  await $.session.start(START)
  await clock.settle()
  await $.tool.call({ tool: 'Read', file_path: `${CWD}\\a.rs` })
  await clock.advance(60_000)

  expect(app.hellos().length).toBe(1)
  expect(app.events()).toEqual([])
  expect(app.status.at(-1)).toBeUndefined()
})

test('el uso y el fin de sesión se envían, y el último envío no espera al temporizador', async ($, on) => {
  const { app, clock } = world(on, { isAppOpen: true })
  on('session.measure', (_$, e) => ({ changed: e.changed }))
  on('session.end', (_$, e) => ({ sessionId: e.sessionId }))
  await $.session.start(START)
  await clock.settle()

  await $.session.measure({
    context: { window: 200_000, tokens: 90_000, percent: 45 },
    rateLimits: [{ kind: 'five_hour', percentUsed: 37, resetsAt: '2026-11-11T20:00:00Z' }],
    cost: { usd: 1.5 },
    changed: ['context'],
  })
  await clock.advance(100)
  expect(app.events().at(-1)).toMatchObject({
    kind: 'usage_snapshot',
    usage: {
      context: { tokens: 90_000, window: 200_000, percent: 45 },
      rate_limits: [{ kind: 'five_hour', percent_used: 37, resets_at: '2026-11-11T20:00:00Z' }],
      cost_usd: 1.5,
    },
  })
  expect(app.status.at(-1)).toBe('◉ cockpit · ctx 45%')

  await $.session.end({ reason: 'other', sessionId: 'session-1', resume: undefined as never })
  expect(app.events().at(-1)).toMatchObject({ kind: 'session_ended', reason: 'other' })
})

test('G3: el diálogo de permisos se señala y se retira, sin decidir nada', async ($, on) => {
  const { app, clock } = world(on, { isAppOpen: true })
  // El motor pide permiso dentro de la herramienta y el usuario tarda 8 s.
  on('classic.PermissionRequest', async () => {
    await clock.sleep(8_000)
    return {}
  })
  on('tool.call', async () => {
    await $.classic.PermissionRequest({ tool_name: 'Write', tool_input: { content: TRAP } })
    await clock.sleep(1_000)
    return { result: {} }
  })
  await $.session.start(START)
  await clock.settle()

  const call = $.tool.call({ tool: 'Write', file_path: `${CWD}\\g3.txt`, content: TRAP })
  await clock.advance(100)
  expect(app.events().map(e => e.kind)).toEqual(['tool_started', 'permission_requested'])
  expect(app.events()[1].tool_use_id).toBe(app.events()[0].tool_use_id)

  await clock.advance(8_000)
  await clock.advance(100)
  expect(app.events().at(-1).kind).toBe('permission_resolved')

  await clock.advance(1_000)
  await call
  await clock.advance(100)
  expect(app.events().map(e => e.kind)).toEqual([
    'tool_started',
    'permission_requested',
    'permission_resolved',
    'tool_finished',
  ])
  expect(JSON.stringify(app.sent).includes(TRAP)).toBe(false)
})

test('G3: un veredicto `ask` de tool.check marca la espera, sin decidir nada', async ($, on) => {
  const { app, clock } = world(on, { isAppOpen: true })
  const kinds = () => app.events().map(e => e.kind).filter(k => k !== 'permission_resolved')
  const checked = new Set<string>()
  const verdicts: Array<{ decision?: string }> = []
  // En un test, 2.1.321 comprueba el permiso por su cuenta y pasa el `ask` por
  // PermissionRequest (que aquí aprueba); 2.1.289 no comprueba nada.
  on('classic.PermissionRequest', () => ({ decision: { behavior: 'allow' as const } }))
  // El motor: Write pide permiso, Read no.
  on('tool.check', (_$, e) => {
    checked.add(e.tool)
    return e.tool === 'Write' ? { decision: 'ask' as const, reason: TRAP } : { decision: 'allow' as const }
  })
  on('tool.call', async (_$, e) => {
    if (!checked.has(e.tool)) {
      // Donde el motor del test no comprueba, se hace aquí como en una llamada
      // real: con el id de la llamada.
      const input = e.tool === 'Write' ? { file_path: `${CWD}\g3.txt`, content: TRAP } : { file_path: `${CWD}\a.rs` }
      verdicts.push((await $.tool.check({ tool: e.tool, input, tool_use_id: e.tool_use_id } as never)) as { decision?: string })
    }
    await clock.sleep(3_000)
    return { result: {} }
  })
  await $.session.start(START)
  await clock.settle()

  const write = $.tool.call({ tool: 'Write', file_path: `${CWD}\g3.txt`, content: TRAP })
  await clock.advance(100)
  expect(kinds()).toEqual(['tool_started', 'permission_requested'])
  expect(app.events()[1].tool_use_id).toBe(app.events()[0].tool_use_id)
  await clock.advance(3_000)
  await write

  const read = $.tool.call({ tool: 'Read', file_path: `${CWD}\a.rs` })
  await clock.advance(3_100)
  await read
  await clock.advance(100)

  // Una sola señal por espera, que se cierra como tarde con el fin de la
  // herramienta; un `allow` no señala nada.
  expect(kinds()).toEqual(['tool_started', 'permission_requested', 'tool_finished', 'tool_started', 'tool_finished'])
  // El mod no decide: las dos herramientas corren y el veredicto sube como lo dio el motor.
  expect(app.events().filter(e => e.kind === 'tool_finished').map(e => e.status)).toEqual(['ok', 'ok'])
  for (const [i, verdict] of verdicts.entries()) expect(verdict.decision).toBe(i === 0 ? 'ask' : 'allow')
  expect(JSON.stringify(app.sent).includes(TRAP)).toBe(false)
})
