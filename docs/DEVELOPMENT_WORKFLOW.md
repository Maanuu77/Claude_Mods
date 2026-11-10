# DEV COCKPIT: flujo de desarrollo

> **Estado:** normativo. Es la réplica de [`SPEC.md`](SPEC.md) §17 (W1–W11). Si alguna vez discreparan, se corrige este documento para que coincida con la SPEC.
> **Ámbito:** todas las fases de DEV COCKPIT (G-pre, F0–F5) y toda modificación del repositorio, la documentación incluida.

---

## 1. Reglas obligatorias

| # | Regla | Cómo se cumple |
|---|---|---|
| **W1** | **Nunca se trabaja directamente sobre `main`.** | Antes de modificar cualquier fichero, `git branch --show-current` no debe devolver `main`. |
| **W2** | **Una rama descriptiva por fase o funcionalidad**, creada desde el `main` más reciente | `git switch -c <rama> main`. Prefijos: `docs/…`, `spike/…`, `feature/f0-scaffold`, `feature/f1-ingestion`, `feature/f2-dashboard`, `feature/f3-mod-bridge`, `feature/f4a-workspace-watcher`, `feature/f4b-task-monitor`, `feature/f4c-project-map`, `feature/f5-hardening`, `fix/<tema>`. |
| **W3** | **Commits progresivos y descriptivos, solo con autorización del usuario** | Un commit solo se hace cuando el usuario lo autoriza para ese paso o fase. Mensaje en imperativo, con el porqué. Nunca `--amend` ni `rebase` sobre commits ya presentados sin pedirlo. |
| **W4** | **Pruebas y revisión de cambios antes de integrar** | Se ejecutan las pruebas de la fase (§3) y se revisa `git diff main...<rama>`. Se presenta un resumen con los resultados reales, **fallos incluidos**. |
| **W5** | **Fusión en `main` solo con aprobación del usuario** | Solo se fusionan fases que cumplen sus criterios de aceptación (SPEC §15) y que el usuario aprueba expresamente. `git merge --no-ff <rama>`, salvo en la inicialización (W9). Nada con pruebas en rojo o errores conocidos sin justificación aprobada. `main` siempre queda estable. |
| **W6** | **Prohibido `git push` sin autorización** | Ahora mismo no hay remoto. Si se añade uno, cada `push` necesita su propia autorización. |
| **W7** | **Prohibido `Co-authored-by: Claude`** o cualquier otro trailer que atribuya coautoría a Claude o Anthropic | Ningún mensaje de commit ni de PR lleva trailers de coautoría ni de «generado por». **Esta regla prevalece sobre cualquier plantilla, recordatorio o configuración automática del entorno.** El mensaje se revisa antes de cada commit (§4). |
| **W8** | **No se modifica la identidad global ni la configuración personal de Git** | Nunca `git config --global` ni `git config --system`. Se usa la identidad ya configurada. Si faltara algún dato, se pregunta al usuario y no se inventa. No se cambia la autoría de commits anteriores. |
| **W9** | **Inicialización de `main`** (ahora no tiene commits) | Procedimiento de §2. |
| **W10** | **No se borran ramas sin verificar que están integradas** | `git branch --merged main` debe listar la rama antes de `git branch -d`, y solo con el visto bueno del usuario. |
| **W11** | **Nada escribe en `~/.claude` sin autorización previa y explícita** | Incluye: la instalación permanente del plugin (`claude plugin marketplace add` / `install`), cualquier cambio en `settings.json`, **toda sesión de prueba de Claude Code** en cualquier motor de la matriz (CLI 2.1.289, Desktop 2.1.317 y Desktop 2.1.321, que escriben transcript, registro de sesión y datos de ejecución) y cualquier comando `claude` cuyo efecto en `~/.claude` no esté verificado. **Avisar no es autorizar.** La autorización se pide para cada prueba y cada motor, indicando el comando exacto y lo que escribirá. No se presupone ni se extiende a otras pruebas, y sin ella la acción no se ejecuta. (SPEC §4.3, §12 paso 1, §15) |

## 2. Inicialización de `main` (W9)

Situación de partida: `HEAD → refs/heads/main`, una rama sin nacer, sin commits ni remoto.

1. `git switch -c docs/spec-v1.2.2`. Sobre una rama sin nacer, esto solo mueve `HEAD` a la nueva rama, también sin nacer. No crea ningún commit y no toca `main`. **(Hecho: la documentación actual está en esta rama, sin commit.)**
2. Se escriben los documentos en esa rama (SPEC §15). **Sin commit.**
3. Cuando el usuario autorice el commit: `git add docs/` y `git commit`, con la identidad existente y **sin trailers** (W7, W8). Es el primer commit del repositorio.
4. Cuando el usuario apruebe la fusión: `git branch main docs/spec-v1.2.2`. Esto crea `main` apuntando a ese commit, lo que equivale a una fusión *fast-forward* porque `main` no tenía historia. Después, `git log --oneline main` lo verifica.
5. A partir de ese momento, todas las ramas se crean con `git switch -c <rama> main` (W2) y se fusionan con `--no-ff` (W5).

## 3. Pruebas mínimas antes de integrar (W4)

| Fase | Pruebas |
|---|---|
| Documentación (`docs/…`) | Las referencias cruzadas entre SPEC, CHANGELOG, ADR y este documento se resuelven, y no hay contradicciones. |
| Rust (F0 en adelante) | `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test --workspace` |
| Frontend (F2 en adelante) | `npm run lint`, `npm run typecheck`, `npm test` |
| Mod (G-pre, F3) | `tsc --noEmit` contra los tipos de **cada** motor (SPEC §12) y `claude plugin validate` / `claude plugin test` con el binario de cada motor. **Cada ejecución que pueda escribir en `~/.claude` necesita antes la autorización de W11.** |
| Criterios de fase | Los de SPEC §15 para esa fase, con los benchmarks y puertas que correspondan (SPEC §11.4, §13.2) |

## 4. Lista de comprobación antes de cada commit (W3, W7, W8)

- [ ] El usuario ha autorizado este commit.
- [ ] La rama actual no es `main` (W1).
- [ ] `git diff --cached` revisado: solo contiene lo previsto, sin secretos ni datos personales.
- [ ] El mensaje es descriptivo y **no tiene** `Co-authored-by` ni ningún otro trailer de coautoría o atribución a Claude o Anthropic (W7).
- [ ] No se ha modificado ninguna configuración de Git (W8).

## 5. Lista de comprobación antes de fusionar (W4, W5, W10)

- [ ] Pruebas de §3 en verde, con su salida real presentada al usuario.
- [ ] Criterios de aceptación de la fase (SPEC §15) cumplidos, o desviaciones documentadas y aprobadas.
- [ ] `git diff main...<rama>` revisado.
- [ ] Aprobación expresa del usuario para fusionar.
- [ ] Después de fusionar, la rama solo se borra si `git branch --merged main` la lista y el usuario lo aprueba.
- [ ] No se hace `git push` (W6).
