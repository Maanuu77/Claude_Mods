# ADR-0008: Lista blanca de metadatos aplicada en tres puntos y política `PRE_HANDSHAKE`

- **Estado:** Aceptada (2026-11-10), SPEC v1.2.2
- **Referencias:** SPEC §5.4, §6.2.1, §10, D1, D2

## Contexto
Los argumentos y resultados de las herramientas pueden contener código, secretos o datos personales. Los transcripts guardan los secretos en texto plano. El usuario fijó tres requisitos:
- solo se capturan métricas y metadatos;
- los argumentos y resultados **nunca** se guardan ni se transmiten completos;
- antes del primer handshake no se transmite ninguna ruta ni ningún título.

## Decisión
- **Lista blanca por herramienta** (SPEC §10). Lo que no está en la lista no se captura. Por ejemplo:
  - `Edit`/`Write` → solo `rel_path`;
  - `Bash` → solo `run_in_background`;
  - MCP → solo `server` y `tool`;
  - herramientas de tareas → `subject` truncado a 120 caracteres (D1).
- **Tres puntos de aplicación independientes:**
  1. `privacy.ts` en el mod, en el momento de la captura;
  2. `deny_unknown_fields` en `HttpIngest`, que rechaza el evento entero;
  3. el `PrivacyFilter` del parser de transcripts.
- **`PRE_HANDSHAKE`:** hasta el primer `hello` correcto de la sesión, el mod no captura títulos ni rutas (`rel_path`, `cwd` y `host` incluidos). Esta política no se cachea entre sesiones. Los huecos que deja los rellena la app desde el transcript, aplicando su propia política.
- **Ajustes:**
  - `capture_task_titles = true` (D1);
  - `capture_shell_program = false` (D2);
  - `record_paths = true`;
  - `retention.path_detail_days = 30` (ADR-0011).
- `meta_json` nunca contiene rutas: el `PrivacyFilter` rechaza las claves de ruta.

## Alternativas consideradas
- **Lista negra (eliminar los campos sensibles conocidos):** cualquier campo nuevo se filtraría. Descartado.
- **Cachear la política autorizada entre sesiones:** contradice el requisito de restricción previa al handshake. Descartado.

## Consecuencias
- **A favor:** defensa en profundidad y ninguna fuga aunque falle uno de los tres puntos.
- **En contra:** antes del handshake se pierde la atribución por ruta. Esos cambios bajan a la regla R2 (confianza `medium`) hasta que el transcript la recupera.

## Validación
- Tests de propiedades con entradas «trampa» (R17).
- E2E de privacidad previa al handshake (SPEC §16, paso 6).
- Test de F3: no sale ninguna ruta ni ningún título antes del primer `hello`.
