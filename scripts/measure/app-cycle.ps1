# B11: ciclo de vida de la app para medir la reconexión del mod (SPEC §5.4, S1–S4).
#
#   app-cycle.ps1 -Action stop    Cierra la app de golpe y borra endpoint.json (como un cierre limpio).
#   app-cycle.ps1 -Action kill    Cierra la app de golpe y deja endpoint.json obsoleto (muerte abrupta).
#   app-cycle.ps1 -Action start   Abre la app y espera a que escriba endpoint.json.
#   app-cycle.ps1 -Action wait    Espera el siguiente hello y da su latencia desde endpoint.json.
#   app-cycle.ps1 -Action log     Resume las peticiones recibidas (sin contenido).
#
# -HttpLog es el fichero de DEV_COCKPIT_HTTP_LOG. La app usa su carpeta real,
# %APPDATA%\dev-cockpit, porque es donde el mod busca endpoint.json.
param(
  [Parameter(Mandatory)] [ValidateSet('stop', 'kill', 'start', 'wait', 'log')] [string]$Action,
  [Parameter(Mandatory)] [string]$HttpLog,
  [string]$Exe = 'target\release\dev-cockpit.exe',
  [int]$TimeoutSeconds = 40
)

$endpoint = Join-Path $env:APPDATA 'dev-cockpit\endpoint.json'
$epoch = [DateTime]::new(1970, 1, 1, 0, 0, 0, [DateTimeKind]::Utc)
function Lines { if (Test-Path $HttpLog) { @(Get-Content $HttpLog) } else { @() } }

switch ($Action) {
  { $_ -in 'stop', 'kill' } {
    Get-Process dev-cockpit -ErrorAction SilentlyContinue | Stop-Process -Force -Confirm:$false
    Start-Sleep -Milliseconds 800
    if ($Action -eq 'stop') { Remove-Item $endpoint -ErrorAction SilentlyContinue }
    "app cerrada; endpoint.json presente: $(Test-Path $endpoint)"
  }
  'start' {
    $before = if (Test-Path $endpoint) { (Get-Item $endpoint).LastWriteTimeUtc } else { $null }
    $env:DEV_COCKPIT_HTTP_LOG = $HttpLog
    Start-Process -FilePath $Exe | Out-Null
    $deadline = (Get-Date).AddSeconds(20)
    while ((Get-Date) -lt $deadline) {
      if ((Test-Path $endpoint) -and ((Get-Item $endpoint).LastWriteTimeUtc -ne $before)) { break }
      Start-Sleep -Milliseconds 100
    }
    $written = [int64]((Get-Item $endpoint).LastWriteTimeUtc - $epoch).TotalMilliseconds
    "app abierta; endpoint.json escrito en $written"
  }
  'wait' {
    $written = [int64]((Get-Item $endpoint).LastWriteTimeUtc - $epoch).TotalMilliseconds
    $deadline = (Get-Date).AddSeconds($TimeoutSeconds)
    $hello = $null
    while ((Get-Date) -lt $deadline -and -not $hello) {
      $hello = Lines | Where-Object { $_ -match '^(\d+) hello ' -and [int64]$Matches[1] -ge $written } | Select-Object -First 1
      if (-not $hello) { Start-Sleep -Milliseconds 200 }
    }
    if (-not $hello) { "sin hello en $TimeoutSeconds s"; exit 1 }
    [void]($hello -match '^(\d+) hello ')
    $at = [int64]$Matches[1]
    $engine = if ($hello -match 'EngineInfo \{ version: "([\d.]+)"') { $Matches[1] } else { '?' }
    $reason = if ($hello -match 'reason: "(\w+)"') { $Matches[1] } else { '?' }
    $policy = if ($hello -match 'AppliedPolicy \{ id: "([\w-]+)"') { $Matches[1] } else { '?' }
    "hello recibido: motor $engine, motivo $reason, política aplicada $policy, $($at - $written) ms después de endpoint.json"
  }
  'log' {
    foreach ($line in Lines) {
      if ($line -match '^(\d+) (hello|events) ') {
        $at = $Matches[1]; $route = $Matches[2]
        $kinds = ([regex]::Matches($line, '\b(ToolStarted|ToolFinished|TurnCompleted|UsageSnapshot|SessionEnded) \{') | ForEach-Object { $_.Groups[1].Value }) -join ', '
        $engine = if ($line -match 'EngineInfo \{ version: "([\d.]+)"') { " motor $($Matches[1])" } else { '' }
        $reason = if ($route -eq 'hello' -and $line -match ' reason: "(\w+)"') { " motivo $($Matches[1])" } else { '' }
        "$at $route$engine$reason $kinds"
      }
    }
  }
}
