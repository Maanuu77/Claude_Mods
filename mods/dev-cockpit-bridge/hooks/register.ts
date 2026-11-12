// DEV COCKPIT: mod puente (SPEC §5). Solo observa.
//
// - Ningún hook bloquea ni reescribe: todos devuelven lo que devuelve `next(e)`.
// - Los hooks solo encolan. El envío va aparte, fuera del camino de la herramienta.
// - Si la app no está, falla en silencio: cola acotada y la sesión no se entera.
// - De los argumentos de una herramienta solo sale la lista blanca de SPEC §10.
//
// Reglas del analizador estático del motor (G-pre):
// - `$` solo se pasa a funciones declaradas al nivel superior de este fichero;
// - `$` se escribe siempre como `$.nombre.método(...)`;
// - el valor de `on(...)` no se guarda.
import type { Register } from 'claude-code'

const MOD_NAME = 'dev-cockpit-bridge'
const MOD_VERSION = '0.1.0'
const SCHEMA = 1
const APP_DIR = 'dev-cockpit'
const QUEUE_MAX = 2_000
const BATCH_MAX = 500
const FLUSH_DELAY_MS = 50
const WATCH_MS = 15_000
const EXTERNAL_PATH = '<externo>'
const PLAN_TOOL = 'mcp__ccd_turn__update_plan'
const SUBJECT_MAX = 120
const LIST_MAX = 200
const AGENTS_MAX = 100

// ---------------------------------------------------------------------------
// Lo que el mod usa del motor. `tsc` comprueba contra los tipos de cada motor
// soportado que su `$` encaja aquí (SPEC §12).
// ---------------------------------------------------------------------------
type Cancel = { cancel: () => void }
type Api = {
  clock: { after: (ms: number, fn: () => void) => Cancel; every: (ms: number, fn: () => void) => Cancel }
  fs: { read: (path: string) => Promise<string>; stat: (path: string) => Promise<unknown> }
  http: {
    fetch: (
      url: string,
      init?: { method?: string; headers?: Record<string, string>; body?: string },
    ) => Promise<{ status: number; text: string }>
  }
  env: { get: (name: string) => Promise<string | undefined> }
  session: {
    id: () => Promise<string>
    version: () => Promise<{ version: string; base?: string }>
    usage: () => Promise<EngineUsage>
  }
  agent: { list: () => Promise<Array<{ id: string; type: string; status: string; parentId?: string }>> }
  ui: { status: (text: string | undefined) => void }
  command: { register: (spec: { name: string; description: string }) => Promise<unknown> }
}

type EngineUsage = {
  context: { tokens?: number; window: number; percent?: number }
  rateLimits: Array<{ kind: string; percentUsed: number; resetsAt?: string }>
  cost?: { usd: number }
}

// ---------------------------------------------------------------------------
// Contrato con la app (crates/cockpit-ingest/src/wire.rs). La app rechaza
// cualquier campo que no esté aquí.
// ---------------------------------------------------------------------------
type Policy = { id: string; capture_task_titles: boolean; record_paths: boolean; capture_shell_program: boolean }
type ToolMeta = {
  rel_path?: string
  host?: string
  ext?: string
  replace_all?: boolean
  run_in_background?: boolean
  program?: string
  subagent_type?: string
}
type UsageWire = {
  context: { tokens?: number; window?: number; percent?: number }
  rate_limits: Array<{ kind: string; percent_used: number; resets_at?: string }>
  cost_usd?: number
}
type WireEvent = { seq: number; t: number } & (
  | { kind: 'tool_started'; tool_use_id: string; tool: string; agent_id?: string; meta: ToolMeta }
  | { kind: 'tool_finished'; tool_use_id: string; status: 'ok' | 'error' | 'interrupted'; agent_id?: string }
  | {
      kind: 'turn_completed'
      turn_id: string
      agent_id?: string
      duration_ms: number
      is_aborted: boolean
      usage?: {
        input_tokens: number
        output_tokens: number
        cache_read_input_tokens: number
        cache_creation_input_tokens: number
        model: string
      }
    }
  | { kind: 'usage_snapshot'; usage: UsageWire }
  | { kind: 'permission_requested'; tool_use_id: string }
  | { kind: 'permission_resolved'; tool_use_id: string }
  | { kind: 'session_ended'; reason?: string }
  | { kind: 'task_op'; tool_use_id?: string; agent_id?: string; source: TaskSource; op: TaskOp }
  | { kind: 'agents'; agents: AgentWire[] }
)
// Task Monitor (SPEC §8). Espejo de crates/cockpit-core/src/tasks.rs.
type TaskSource = 'task_tools' | 'todo_write' | 'desktop_plan'
type TaskStatus = 'pending' | 'in_progress' | 'completed' | 'deleted'
type TaskOp =
  | { type: 'create'; task_id: string; subject?: string }
  | { type: 'update'; task_id: string; status?: TaskStatus; subject?: string; owner?: string; blocked_by?: string[] }
  | { type: 'complete'; task_id: string }
  | { type: 'replace'; items: Array<{ subject?: string; status: TaskStatus }> }
  | { type: 'advance' }
  | { type: 'mark'; done?: number[]; start?: number }
type AgentWire = { id: string; type: string; status: string; parent_id?: string }
type Pending = WireEvent extends infer E ? (E extends WireEvent ? Omit<E, 'seq'> : never) : never
type RunningTool = {
  tool_use_id: string
  tool: string
  agent_id?: string
  started_at: number
  meta: ToolMeta
  /** El diálogo de permisos está abierto para esta llamada. No se transmite. */
  isAwaitingPermission?: boolean
}

/** La política más restrictiva: sin títulos y sin rutas de ningún tipo (SPEC §5.4). */
const PRE_HANDSHAKE: Policy = {
  id: 'pre_handshake',
  capture_task_titles: false,
  record_paths: false,
  capture_shell_program: false,
}

type Connection = 'disconnected' | 'connecting' | 'connected' | 'unsupported'

// Estado del módulo. Se pierde en cada recarga, y por eso cada carga es una
// instancia nueva para la app.
const st = {
  instanceId: newId(),
  seq: 0,
  helloSeq: 0,
  queue: [] as WireEvent[],
  dropped: 0,
  policy: PRE_HANDSHAKE,
  connection: 'disconnected' as Connection,
  hasConnectedOnce: false,
  endpointPath: undefined as string | undefined,
  endpoint: undefined as { port: number; token: string } | undefined,
  sessionId: undefined as string | undefined,
  engine: undefined as { version: string; base?: string } | undefined,
  /** Raíz para relativizar rutas. Nunca se transmite. */
  cwd: undefined as string | undefined,
  surface: undefined as string | undefined,
  isInteractive: false,
  startedAt: Date.now(),
  running: new Map<string, RunningTool>(),
  contextPercent: undefined as number | undefined,
  flushTimer: undefined as Cancel | undefined,
  watchTimer: undefined as Cancel | undefined,
  isFlushing: false,
  connecting: undefined as Promise<boolean> | undefined,
  lastSentAt: undefined as number | undefined,
  lastError: undefined as string | undefined,
  sent: 0,
  hookCalls: 0,
  hookOwnMsMax: 0,
  /** Último `agents` enviado, para no repetirlo si no cambia. */
  lastAgents: '',
  /** Subagentes vistos por los hooks clásicos y que `$.agent.list()` ya no lista. */
  hookAgents: new Map<string, AgentWire>(),
}

export const register: Register = on => {
  on('session.start', async ($, e, next) => {
    const started = await next(e)
    st.cwd = started.cwd
    st.surface = e.surface ?? undefined
    st.isInteractive = e.isInteractive
    void boot($)
    return started
  })

  // Live Tool Monitor: `tool_started` antes de ejecutar, `tool_finished` al acabar.
  on('tool.call', async ($, e, next) => {
    const t0 = Date.now()
    const call = e as unknown as { tool: string; tool_use_id: string; agentId?: string } & Record<string, unknown>
    const entry: RunningTool = {
      tool_use_id: call.tool_use_id,
      tool: call.tool,
      agent_id: call.agentId,
      started_at: t0,
      meta: extractMeta(call.tool, call, st.policy, st.cwd),
    }
    st.running.set(entry.tool_use_id, entry)
    enqueue({ kind: 'tool_started', t: t0, tool_use_id: entry.tool_use_id, tool: entry.tool, agent_id: entry.agent_id, meta: entry.meta })
    scheduleFlush($)
    noteHook(t0)
    let status: 'ok' | 'error' | 'interrupted' = 'error'
    try {
      const ran = await next(e)
      status = ran.deny === undefined && ran.isError !== true ? 'ok' : 'error'
      if (status === 'ok') noteTaskOp($, entry, call, (ran as { result?: unknown }).result)
      return ran
    } catch (err) {
      status = next.signal.aborted ? 'interrupted' : 'error'
      throw err
    } finally {
      const t1 = Date.now()
      st.running.delete(entry.tool_use_id)
      enqueue({ kind: 'tool_finished', t: t1, tool_use_id: entry.tool_use_id, status, agent_id: entry.agent_id })
      scheduleFlush($)
      noteHook(t1)
    }
    // Si algo de este hook falla, la herramienta sigue su curso sin el mod.
  }).catch((_$, e, next) => next(e))

  // G3: el motor llama a `tool.call` antes de pedir permiso, así que el tiempo de
  // la herramienta incluye el diálogo. Hay dos señales y ninguna decide nada.
  //
  // 1) `tool.check`: el veredicto del motor. Se lee y se devuelve tal cual (un
  //    veredicto que sube intacto no cuenta como del mod). Si es `ask`, alguien
  //    tiene que contestar. El motor no avisa de cuándo contesta: la espera se
  //    cierra con `permission_resolved` (si llega) o con el fin de la herramienta.
  on('tool.check', async ($, e, next) => {
    const verdict = await next(e)
    try {
      const id = (e as { tool_use_id?: string }).tool_use_id
      const asked = id === undefined ? undefined : st.running.get(id)
      if (verdict.decision === 'ask' && asked && !asked.isAwaitingPermission) {
        asked.isAwaitingPermission = true
        enqueue({ kind: 'permission_requested', t: Date.now(), tool_use_id: asked.tool_use_id })
        scheduleFlush($)
      }
    } catch {
      // Observar no puede cambiar el veredicto.
    }
    return verdict
  })

  // 2) `classic.PermissionRequest`: solo llega donde no contesta una persona en
  //    el diálogo (subagentes en segundo plano, `-p`). Ahí sí marca el final.
  on('classic.PermissionRequest', async ($, e, next) => {
    const sameTool = [...st.running.values()].reverse().filter(r => r.tool === e.tool_name)
    const waiting = sameTool.find(r => r.isAwaitingPermission) ?? sameTool[0]
    if (!waiting) return next(e)
    if (!waiting.isAwaitingPermission) {
      waiting.isAwaitingPermission = true
      enqueue({ kind: 'permission_requested', t: Date.now(), tool_use_id: waiting.tool_use_id })
      scheduleFlush($)
    }
    try {
      return await next(e)
    } finally {
      waiting.isAwaitingPermission = false
      if (st.running.has(waiting.tool_use_id)) {
        enqueue({ kind: 'permission_resolved', t: Date.now(), tool_use_id: waiting.tool_use_id })
        scheduleFlush($)
      }
    }
    // Si este hook falla, la petición de permiso sigue su curso sin el mod.
  }).catch((_$, e, next) => next(e))

  // Task Monitor (SPEC §8): tareas creadas y terminadas fuera de una llamada del
  // modelo (compañeros de equipo) y el estado de los subagentes, por evento.
  on('classic.TaskCreated', async ($, e, next) => {
    const done = await next(e)
    const id = taskId(e.task_id)
    if (id !== undefined) {
      enqueue({ kind: 'task_op', t: Date.now(), source: 'task_tools', op: { type: 'create', task_id: id, subject: title(e.task_subject, st.policy) } })
      scheduleFlush($)
    }
    return done
  }).catch((_$, e, next) => next(e))

  on('classic.TaskCompleted', async ($, e, next) => {
    const done = await next(e)
    const id = taskId(e.task_id)
    if (id !== undefined) {
      enqueue({ kind: 'task_op', t: Date.now(), source: 'task_tools', op: { type: 'complete', task_id: id } })
      scheduleFlush($)
    }
    return done
  }).catch((_$, e, next) => next(e))

  on('classic.SubagentStart', async ($, e, next) => {
    const done = await next(e)
    noteHookAgent(e.agent_id, e.agent_type, 'running')
    void sendAgents($)
    return done
  }).catch((_$, e, next) => next(e))

  on('classic.SubagentStop', async ($, e, next) => {
    const done = await next(e)
    noteHookAgent(e.agent_id, e.agent_type, 'completed')
    void sendAgents($)
    return done
  }).catch((_$, e, next) => next(e))

  on('turn.start', async ($, e, next) => {
    const started = await next(e)
    void sendAgents($)
    return started
  })

  on('turn.complete', async ($, e, next) => {
    const done = await next(e)
    void sendAgents($)
    const u = e.usage
    enqueue({
      kind: 'turn_completed',
      t: Date.now(),
      turn_id: e.turnId,
      agent_id: e.agentId,
      duration_ms: Math.max(0, Math.round(e.durationMs)),
      is_aborted: e.isAborted,
      usage: u && {
        input_tokens: u.input_tokens,
        output_tokens: u.output_tokens,
        cache_read_input_tokens: u.cache_read_input_tokens,
        cache_creation_input_tokens: u.cache_creation_input_tokens,
        model: u.model,
      },
    })
    scheduleFlush($)
    return done
  })

  // Contexto, límites y coste llegan por push: no hay sondeo (SPEC §11.3).
  on('session.measure', async ($, e, next) => {
    const measured = await next(e)
    st.contextPercent = e.context.percent
    enqueue({ kind: 'usage_snapshot', t: Date.now(), usage: toUsage(e) })
    scheduleFlush($)
    showStatus($)
    return measured
  })

  on('session.end', async ($, e, next) => {
    enqueue({ kind: 'session_ended', t: Date.now(), reason: ident(String(e.reason), 48) })
    // Último envío, dentro del margen que el motor da a `session.end`.
    await flush($).catch(() => undefined)
    return next(e)
  })

  on('command.run', { command: 'cockpit' }, async () => ({ text: diagnostics() }))
}

// ---------------------------------------------------------------------------
// Arranque y conexión (SPEC §5.4)
// ---------------------------------------------------------------------------

async function boot($: Api): Promise<void> {
  try {
    st.sessionId = await $.session.id()
    st.engine = await $.session.version()
    const appData = await $.env.get('APPDATA')
    if (appData) st.endpointPath = `${appData}\\${APP_DIR}\\endpoint.json`
  } catch (err) {
    st.lastError = errText(err)
  }
  try {
    await $.command.register({ name: 'cockpit', description: 'DEV COCKPIT: estado de la conexión con la app' })
  } catch (err) {
    st.lastError = errText(err)
  }
  await connect($)
  await flush($)
}

/** Lee el endpoint y hace el handshake. Solo hay un intento en vuelo. */
function connect($: Api): Promise<boolean> {
  if (st.connection === 'unsupported') return Promise.resolve(false)
  st.connecting ??= handshake($).finally(() => {
    st.connecting = undefined
  })
  return st.connecting
}

async function handshake($: Api): Promise<boolean> {
  if (!st.endpointPath || !st.sessionId || !st.engine) return false
  st.connection = 'connecting'
  try {
    // El endpoint se vuelve a leer en cada intento: la app rota puerto y token.
    const endpoint = JSON.parse(await $.fs.read(st.endpointPath)) as { port?: unknown; token?: unknown }
    if (typeof endpoint.port !== 'number' || typeof endpoint.token !== 'string') throw new Error('endpoint inválido')
    st.endpoint = { port: endpoint.port, token: endpoint.token }

    const usage = await $.session.usage().then(toUsage, () => undefined)
    const agents = await $.agent.list().then(
      list => list.map(a => ({ id: a.id, type: a.type, status: a.status, parent_id: a.parentId })),
      () => [],
    )
    st.helloSeq += 1
    const response = await post($, '/v1/hello', {
      schema: SCHEMA,
      mod: { name: MOD_NAME, version: MOD_VERSION, instance_id: st.instanceId },
      engine: { version: st.engine.version, base: st.engine.base },
      session: {
        id: st.sessionId,
        surfaces: st.surface ? [st.surface] : [],
        is_interactive: st.isInteractive,
        started_at: st.startedAt,
      },
      // El primer handshake de la instancia es `startup`, llegue cuando llegue.
      reason: st.hasConnectedOnce ? 'reconnect' : 'startup',
      hello_seq: st.helloSeq,
      applied_policy: st.policy,
      snapshot: {
        usage,
        agents,
        running_tools: [...st.running.values()].map(r => ({
          tool_use_id: r.tool_use_id,
          tool: r.tool,
          agent_id: r.agent_id,
          started_at: r.started_at,
          meta: r.meta,
        })),
      },
      queue: { pending: st.queue.length, dropped: st.dropped, next_seq: st.seq + 1 },
    })
    if (response.status !== 200) throw new Error(`hello ${response.status}`)
    const hello = JSON.parse(response.text) as {
      privacy: { policy_id: string; capture_task_titles: boolean; record_paths: boolean; capture_shell_program: boolean }
      events: { ack_seq: number }
      engine_supported: boolean
    }
    if (!hello.engine_supported) {
      // Motor por debajo del mínimo: esta sesión se queda en modo degradado.
      st.connection = 'unsupported'
      st.queue.length = 0
      stopWatch()
      showStatus($)
      return false
    }
    // La política autorizada vale para lo que se capture desde ahora (SPEC §5.4).
    st.policy = {
      id: hello.privacy.policy_id,
      capture_task_titles: hello.privacy.capture_task_titles,
      record_paths: hello.privacy.record_paths,
      capture_shell_program: hello.privacy.capture_shell_program,
    }
    acknowledge(hello.events.ack_seq)
    st.connection = 'connected'
    st.hasConnectedOnce = true
    st.lastError = undefined
    stopWatch()
    showStatus($)
    return true
  } catch (err) {
    st.lastError = errText(err)
    disconnect($)
    return false
  }
}

/** Sin conexión: una sola vigilancia ligera hasta que la app aparezca. */
function disconnect($: Api): void {
  st.connection = 'disconnected'
  st.endpoint = undefined
  showStatus($)
  if (st.watchTimer || !st.endpointPath) return
  st.watchTimer = $.clock.every(WATCH_MS, () => {
    void watchTick($)
  })
}

async function watchTick($: Api): Promise<void> {
  if (st.connection !== 'disconnected' || !st.endpointPath) return
  try {
    await $.fs.stat(st.endpointPath)
  } catch {
    return // la app sigue cerrada
  }
  if (await connect($)) await flush($)
}

function stopWatch(): void {
  st.watchTimer?.cancel()
  st.watchTimer = undefined
}

// ---------------------------------------------------------------------------
// Cola y envío (SPEC §5.3)
// ---------------------------------------------------------------------------

function enqueue(event: Pending): void {
  if (st.connection === 'unsupported') return
  if (st.queue.length >= QUEUE_MAX) {
    // Se sacrifica primero lo que no es de herramientas; si no hay, lo más antiguo.
    const victim = st.queue.findIndex(q => q.kind !== 'tool_started' && q.kind !== 'tool_finished')
    st.queue.splice(victim === -1 ? 0 : victim, 1)
    st.dropped += 1
  }
  st.seq += 1
  st.queue.push({ ...event, seq: st.seq } as WireEvent)
}

/** Programa un único envío. Con la cola vacía no queda ningún temporizador. */
function scheduleFlush($: Api): void {
  if (st.flushTimer || st.isFlushing || st.queue.length === 0 || st.connection === 'unsupported') return
  st.flushTimer = $.clock.after(FLUSH_DELAY_MS, () => {
    st.flushTimer = undefined
    void flush($)
  })
}

async function flush($: Api): Promise<void> {
  if (st.isFlushing || st.queue.length === 0 || !st.sessionId) return
  st.isFlushing = true
  try {
    for (let attempt = 0; attempt < 2 && st.queue.length > 0; attempt++) {
      if (st.connection !== 'connected' && !(await connect($))) return
      const batch = st.queue.slice(0, BATCH_MAX)
      const response = await post($, '/v1/events', {
        schema: SCHEMA,
        instance_id: st.instanceId,
        session_id: st.sessionId,
        events: batch,
      })
      if (response.status === 200) {
        acknowledge((JSON.parse(response.text) as { ack_seq: number }).ack_seq)
        st.sent += batch.length
        st.lastSentAt = Date.now()
        attempt = -1 // sigue con el resto de la cola
      } else if (response.status === 401 || response.status === 409) {
        // La app se ha reiniciado: token nuevo o instancia desconocida.
        st.connection = 'disconnected'
      } else {
        // La app ha rechazado el lote: reintentarlo no cambia nada.
        st.lastError = `events ${response.status}`
        acknowledge(batch.at(-1)?.seq ?? 0)
        st.dropped += batch.length
        return
      }
    }
  } catch (err) {
    st.lastError = errText(err)
    disconnect($)
  } finally {
    st.isFlushing = false
  }
}

function acknowledge(ackSeq: number): void {
  st.queue = st.queue.filter(q => q.seq > ackSeq)
}

function post($: Api, path: string, body: unknown): Promise<{ status: number; text: string }> {
  if (!st.endpoint) return Promise.reject(new Error('sin endpoint'))
  return $.http.fetch(`http://127.0.0.1:${st.endpoint.port}${path}`, {
    method: 'POST',
    headers: { Authorization: `Bearer ${st.endpoint.token}`, 'Content-Type': 'application/json' },
    body: JSON.stringify(body),
  })
}

// ---------------------------------------------------------------------------
// Indicador y diagnóstico
// ---------------------------------------------------------------------------

function showStatus($: Api): void {
  if (st.connection !== 'connected') {
    $.ui.status(st.connection === 'unsupported' ? undefined : '○ cockpit')
    return
  }
  const context = st.contextPercent === undefined ? '' : ` · ctx ${Math.round(st.contextPercent)}%`
  $.ui.status(`◉ cockpit${context}`)
}

function diagnostics(): string {
  const timers = (st.flushTimer ? 1 : 0) + (st.watchTimer ? 1 : 0)
  const lines = [
    `DEV COCKPIT · mod ${MOD_VERSION} · motor ${st.engine?.version ?? 'desconocido'}`,
    `conexión: ${st.connection}${st.lastError ? ` (último error: ${st.lastError})` : ''}`,
    `política: ${st.policy.id}`,
    `cola: ${st.queue.length} pendientes · ${st.sent} enviados · ${st.dropped} descartados`,
    `último envío: ${st.lastSentAt ? `hace ${Math.round((Date.now() - st.lastSentAt) / 1000)} s` : 'ninguno'}`,
    `herramientas en curso: ${st.running.size}`,
    `temporizadores activos: ${timers}`,
    `hooks: ${st.hookCalls} llamadas · coste propio máximo ${st.hookOwnMsMax} ms`,
  ]
  return lines.join('\n')
}

function noteHook(startedAt: number): void {
  st.hookCalls += 1
  st.hookOwnMsMax = Math.max(st.hookOwnMsMax, Date.now() - startedAt)
}

// ---------------------------------------------------------------------------
// Task Monitor (SPEC §8)
// ---------------------------------------------------------------------------

/** Encola la operación que declara una llamada terminada sin error. */
function noteTaskOp($: Api, entry: RunningTool, input: Record<string, unknown>, result: unknown): void {
  try {
    const declared = taskOp(entry.tool, input, result, st.policy, entry.agent_id !== undefined)
    if (!declared) return
    enqueue({ kind: 'task_op', t: Date.now(), tool_use_id: entry.tool_use_id, agent_id: entry.agent_id, ...declared })
    scheduleFlush($)
  } catch {
    // Observar una lista de tareas nunca puede romper la herramienta.
  }
}

/** Claves de `tool.call` que no son argumentos de la herramienta. */
const RESERVED_KEYS = new Set(['tool', 'tool_use_id', 'consent', 'agentId', 'requestMeta'])
/** Claves conocidas de `update_plan` (fixtures de G6). */
const PLAN_KEYS = new Set(['items', 'advance', 'done', 'start', 'scope'])

function taskOp(
  tool: string,
  input: Record<string, unknown>,
  result: unknown,
  policy: Policy,
  isSubagent: boolean,
): { source: TaskSource; op: TaskOp } | undefined {
  switch (tool) {
    case 'TaskCreate': {
      // El id lo asigna el motor: sale del resultado.
      const task = (result as { task?: { id?: unknown; subject?: unknown } } | undefined)?.task
      const id = taskId(task?.id)
      if (id === undefined) return undefined
      return { source: 'task_tools', op: { type: 'create', task_id: id, subject: title(text(input.subject) ?? text(task?.subject), policy) } }
    }
    case 'TaskUpdate': {
      const id = taskId(input.taskId)
      if (id === undefined) return undefined
      const status = input.status === undefined ? undefined : taskStatus(input.status)
      if (status === null) return undefined
      const owner = input.owner === undefined ? undefined : ownerName(input.owner)
      if (owner === null) return undefined
      const blockers = Array.isArray(input.addBlockedBy) ? input.addBlockedBy.map(taskId).filter(isDefined) : []
      return {
        source: 'task_tools',
        op: {
          type: 'update',
          task_id: id,
          status,
          subject: title(text(input.subject), policy),
          owner,
          blocked_by: blockers.length > 0 ? [...new Set(blockers)].slice(0, 32) : undefined,
        },
      }
    }
    case 'TodoWrite': {
      // La lista de un subagente es suya, no la de la sesión.
      if (isSubagent) return undefined
      const items = listItems(input.todos, policy)
      return items && { source: 'todo_write', op: { type: 'replace', items } }
    }
    case PLAN_TOOL: {
      // El Desktop rechaza `update_plan` en los subagentes.
      if (isSubagent) return undefined
      const keys = Object.keys(input).filter(k => !RESERVED_KEYS.has(k))
      if (keys.some(k => !PLAN_KEYS.has(k))) return undefined // esquema nuevo: que decida el transcript
      const forms = [input.items !== undefined, input.advance !== undefined, input.done !== undefined || input.start !== undefined]
      if (forms.filter(Boolean).length !== 1) return undefined
      if (input.items !== undefined) {
        const items = listItems(input.items, policy)
        return items && items.length > 0 ? { source: 'desktop_plan', op: { type: 'replace', items } } : undefined
      }
      if (input.advance !== undefined) return input.advance === true ? { source: 'desktop_plan', op: { type: 'advance' } } : undefined
      const done = input.done === undefined ? [] : Array.isArray(input.done) ? input.done.map(step) : [undefined]
      const start = input.start === undefined ? undefined : step(input.start)
      if (done.some(n => n === undefined) || (input.start !== undefined && start === undefined)) return undefined
      return { source: 'desktop_plan', op: { type: 'mark', done: [...new Set(done as number[])], start } }
    }
    default:
      return undefined
  }
}

function listItems(value: unknown, policy: Policy): Array<{ subject?: string; status: TaskStatus }> | undefined {
  if (!Array.isArray(value) || value.length > LIST_MAX) return undefined
  const items: Array<{ subject?: string; status: TaskStatus }> = []
  for (const raw of value) {
    const item = (raw ?? {}) as { content?: unknown; status?: unknown }
    const status = taskStatus(item.status)
    if (status === null || status === 'deleted') return undefined
    // `activeForm` y cualquier otro campo no se leen nunca.
    items.push({ subject: title(text(item.content), policy), status })
  }
  return items
}

function taskStatus(value: unknown): TaskStatus | null {
  return value === 'pending' || value === 'in_progress' || value === 'completed' || value === 'deleted' ? value : null
}

function step(value: unknown): number | undefined {
  return typeof value === 'number' && Number.isInteger(value) && value >= 1 ? value : undefined
}

/** Id de tarea: un identificador corto, nunca texto libre. */
function taskId(value: unknown): string | undefined {
  const id = typeof value === 'string' ? value.trim() : typeof value === 'number' && Number.isInteger(value) ? String(value) : undefined
  return id !== undefined && /^[A-Za-z0-9_.:-]{1,64}$/.test(id) ? id : undefined
}

function ownerName(value: unknown): string | null {
  const name = typeof value === 'string' ? value.trim() : ''
  return /^[A-Za-z0-9_.:@-]{1,64}$/.test(name) ? name : null
}

/** Título de tarea (D1): una línea de 120 caracteres, solo si la política lo permite. */
function title(value: string | undefined, policy: Policy): string | undefined {
  if (!policy.capture_task_titles || value === undefined) return undefined
  const line = value.replace(/[\s\p{Cc}]+/gu, ' ').trim()
  const short = Array.from(line).slice(0, SUBJECT_MAX).join('')
  return short === '' ? undefined : short
}

function isDefined<T>(value: T | undefined): value is T {
  return value !== undefined
}

function noteHookAgent(id: unknown, type: unknown, status: string): void {
  const clean = taskId(id)
  if (clean === undefined) return
  st.hookAgents.set(clean, { id: clean, type: typeof type === 'string' ? ident(type, 64) : 'agent', status })
}

/** `$.agent.list()`, solo si ha cambiado desde el último envío (A4). */
async function sendAgents($: Api): Promise<void> {
  let listed: AgentWire[]
  try {
    listed = (await $.agent.list()).map(a => ({ id: a.id, type: a.type, status: a.status, parent_id: a.parentId }))
  } catch {
    return
  }
  // Lo que vieron los hooks y la lista ya no tiene (el motor suelta sus agentes).
  const ids = new Set(listed.map(a => a.id))
  const agents = [...listed, ...[...st.hookAgents.values()].filter(a => !ids.has(a.id))].slice(0, AGENTS_MAX)
  const key = JSON.stringify(agents)
  if (key === st.lastAgents) return
  st.lastAgents = key
  enqueue({ kind: 'agents', t: Date.now(), agents })
  scheduleFlush($)
}

// ---------------------------------------------------------------------------
// Lista blanca de metadatos (SPEC §10). Espejo de crates/cockpit-core/src/privacy.rs.
// ---------------------------------------------------------------------------

function extractMeta(tool: string, input: Record<string, unknown>, policy: Policy, root: string | undefined): ToolMeta {
  const meta: ToolMeta = {}
  switch (tool) {
    case 'Read':
    case 'Edit':
    case 'MultiEdit':
    case 'Write':
    case 'NotebookEdit': {
      const path = text(input.file_path) ?? text(input.notebook_path)
      if (policy.record_paths && path !== undefined) {
        const rel = relativize(path, root)
        meta.rel_path = rel ?? EXTERNAL_PATH
        if (rel === undefined) meta.ext = extension(path)
      }
      if (typeof input.replace_all === 'boolean') meta.replace_all = input.replace_all
      break
    }
    case 'Bash':
    case 'PowerShell': {
      const background = flag(input.run_in_background)
      if (background !== undefined) meta.run_in_background = background
      if (policy.capture_shell_program) {
        const program = shellProgram(text(input.command))
        if (program !== undefined) meta.program = program
      }
      break
    }
    case 'WebFetch': {
      if (policy.record_paths) {
        const host = urlHost(text(input.url))
        if (host !== undefined) meta.host = host
      }
      break
    }
    case 'Agent': {
      const kind = text(input.subagent_type)
      if (kind !== undefined) meta.subagent_type = ident(kind, 64)
      const background = flag(input.run_in_background)
      if (background !== undefined) meta.run_in_background = background
      break
    }
    // Grep, Glob, WebSearch, MCP, tareas y cualquier otra: solo el nombre.
  }
  return meta
}

function text(value: unknown): string | undefined {
  return typeof value === 'string' ? value : undefined
}

function flag(value: unknown): boolean | undefined {
  if (typeof value === 'boolean') return value
  return value === 'true' ? true : value === 'false' ? false : undefined
}

function isAbsolute(path: string): boolean {
  return path.startsWith('/') || /^[A-Za-z]:/.test(path)
}

/** Ruta relativa a `root` con `/`, o `undefined` si queda fuera o no se sabe. */
function relativize(path: string, root: string | undefined): string | undefined {
  const normalized = path.replace(/\\/g, '/')
  let rel: string
  if (isAbsolute(normalized)) {
    if (!root) return undefined
    const base = root.replace(/\\/g, '/').replace(/\/+$/, '')
    if (base === '' || normalized.slice(0, base.length).toLowerCase() !== base.toLowerCase()) return undefined
    if (normalized[base.length] !== '/') return undefined
    rel = normalized.slice(base.length + 1)
  } else {
    rel = normalized.replace(/^(\.\/)+/, '')
  }
  if (rel === '' || rel.split('/').includes('..')) return undefined
  return rel
}

function extension(path: string): string {
  const name = path.split(/[\\/]/).pop() ?? ''
  const dot = name.lastIndexOf('.')
  const ext = dot > 0 ? name.slice(dot + 1) : ''
  return /^[A-Za-z0-9]{1,12}$/.test(ext) ? `.${ext.toLowerCase()}` : ''
}

/** Nombre del ejecutable: el primer token, sin directorio ni argumentos. */
function shellProgram(command: string | undefined): string | undefined {
  const token = command?.trim().split(/\s+/)[0]
  // `VAR=valor cmd` empieza por una asignación, que puede llevar un secreto.
  if (!token || token.includes('=')) return undefined
  const name = token.replace(/^["'&(]+|["'&)]+$/g, '').split(/[\\/]/).pop() ?? ''
  return /^[A-Za-z0-9._+-]{1,64}$/.test(name) ? name : undefined
}

/** Host en minúsculas, sin credenciales, puerto, ruta ni consulta. */
function urlHost(url: string | undefined): string | undefined {
  const rest = url?.trim().split('://')[1]
  if (rest === undefined) return undefined
  const hostPort = (rest.split(/[/?#]/)[0] ?? '').split('@').pop() ?? ''
  const host = (hostPort.startsWith('[') ? hostPort.slice(1).split(']')[0] : hostPort.split(':')[0]) ?? ''
  return /^[A-Za-z0-9.:-]{1,253}$/.test(host) ? host.toLowerCase() : undefined
}

function toUsage(usage: EngineUsage): UsageWire {
  return {
    context: { tokens: usage.context.tokens, window: usage.context.window, percent: usage.context.percent },
    rate_limits: usage.rateLimits.map(r => ({ kind: r.kind, percent_used: r.percentUsed, resets_at: r.resetsAt })),
    cost_usd: usage.cost?.usd,
  }
}

function ident(value: string, max: number): string {
  return value.replace(/[^A-Za-z0-9_.:-]/g, '').slice(0, max)
}

function errText(err: unknown): string {
  // Solo el nombre del error: el mensaje puede llevar rutas.
  return err instanceof Error ? ident(err.name, 40) : 'error'
}

function newId(): string {
  const hex = (n: number) =>
    Array.from({ length: n }, () => Math.floor(Math.random() * 16).toString(16)).join('')
  return `${hex(8)}-${hex(4)}-4${hex(3)}-${'89ab'[Math.floor(Math.random() * 4)]}${hex(3)}-${hex(12)}`
}
