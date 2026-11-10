// DEV COCKPIT, sonda G-pre (desechable), módulo de eventos. SPEC §13.2 G1.
// Va separado de mod-core a propósito: si un motor rechaza un nombre de evento
// y no carga este módulo, la sonda principal sigue funcionando. Que este módulo
// cargue (aparece results/events-<versión>.json) prueba que el motor acepta
// registrar los tres eventos. Que se disparen requiere un turno del modelo
// (session.measure) o una tarea (classic.Task*): es la prueba opcional T-x.
// Reglas del analizador: `on(...)` como sentencia y `$` solo hacia funciones
// del nivel superior (ver mod-core).
import type { Register } from 'claude-code'

const fired: Record<string, number> = {}
const report: Record<string, unknown> = {
  probe: 'events',
  schema: 2,
  loadedAt: Date.now(),
  registered: ['session.measure', 'classic.TaskCreated', 'classic.TaskCompleted'],
  fired,
}

export const register: Register = on => {
  on('session.measure', ($, e, next) => { fired['session.measure'] = (fired['session.measure'] ?? 0) + 1; void persist($); return next(e) })
  on('classic.TaskCreated', ($, e, next) => { fired['classic.TaskCreated'] = (fired['classic.TaskCreated'] ?? 0) + 1; void persist($); return next(e) })
  on('classic.TaskCompleted', ($, e, next) => { fired['classic.TaskCompleted'] = (fired['classic.TaskCompleted'] ?? 0) + 1; void persist($); return next(e) })
  on('session.start', async ($, e, next) => {
    const started = await next(e)
    void persist($)
    return started
  })
}

// eslint-disable-next-line @typescript-eslint/no-explicit-any
async function persist($: any): Promise<void> {
  try {
    const v = await $.session.version()
    report.engine = v
    report.updatedAt = Date.now()
    const base = String($.plugin.root).replace(/[\\/]\.claude-plugin[\\/]?$/, '')
    await $.fs.write(`${base}/../results/events-${v?.version ?? 'unknown'}.json`, JSON.stringify(report, null, 2))
  } catch { /* la sonda core informa del fallo de fs.write */ }
}
