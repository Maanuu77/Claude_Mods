# B6/B7 (SPEC §11.4): lanza DEV COCKPIT con carpetas de prueba y ejecuta fs-ops.mjs.
#
# La app no lee ~/.claude ni la base de datos real: todas las rutas se redirigen
# con las variables DEV_COCKPIT_*. El CSV de latencias queda en <Out>\perf.csv
# con una fila por pintado: evento,pintado,latencia,tipo (`file` = B6, `numstat` = B7).
#
#   powershell -File scripts\measure\workspace-cycle.ps1 [-Exe target\release\dev-cockpit.exe] [-Out <carpeta>]
param(
  [string]$Exe = "target\release\dev-cockpit.exe",
  [string]$Out = (Join-Path $env:TEMP "dev-cockpit-b6b7")
)
$ErrorActionPreference = "Stop"
if (Test-Path $Out) { Remove-Item -Recurse -Force $Out }
New-Item -ItemType Directory -Force $Out | Out-Null
$env:DEV_COCKPIT_DB = Join-Path $Out "db\cockpit.db"
$env:DEV_COCKPIT_SESSIONS_DIR = Join-Path $Out "sessions"
$env:DEV_COCKPIT_PROJECTS_DIR = Join-Path $Out "projects"
$env:DEV_COCKPIT_PERF_LOG = Join-Path $Out "perf.csv"
New-Item -ItemType Directory -Force $env:DEV_COCKPIT_SESSIONS_DIR, $env:DEV_COCKPIT_PROJECTS_DIR | Out-Null

$app = Start-Process -FilePath $Exe -PassThru
Start-Sleep -Seconds 5
try {
  # La ventana tiene que estar visible: con ella oculta no se publica nada (SPEC §6.5).
  node (Join-Path $PSScriptRoot "fs-ops.mjs") (Join-Path $Out "work") $env:DEV_COCKPIT_SESSIONS_DIR
} finally {
  Stop-Process -Id $app.Id -ErrorAction SilentlyContinue
}
"CSV: $env:DEV_COCKPIT_PERF_LOG"
