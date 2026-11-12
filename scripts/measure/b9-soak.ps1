# B9: prueba de larga duración de DEV COCKPIT (SPEC §11.2 y §11.4).
#
# Arranca la app instalada, la deja trabajar con el uso real del equipo y muestrea
# cada segundo la CPU y la memoria SOLO de sus procesos (C2): el proceso Rust y su
# grupo WebView2, identificado por dos métodos que deben coincidir:
#   1. GetProcessInfos del entorno WebView2 de la app (DEV_COCKPIT_PROCS_LOG).
#   2. Árbol de procesos: msedgewebview2.exe descendientes de dev-cockpit.exe cuyo
#      CommandLine contiene el --user-data-dir propio.
# El conjunto se recalcula en cada muestra (WebView2 puede recrear procesos).
# `crashpad-handler` no lo devuelve el método 1: se suma aparte, fuera de la
# comparación.
#
# Cada muestra se escribe al momento en muestras.csv: si la prueba se corta, lo
# medido hasta ahí no se pierde. Al terminar escribe resumen.txt.
#
# Criterio (§11.2): crecimiento de RAM < 10 % y 0 pánicos. Crecimiento = media de
# la memoria privada total en la última hora frente a la media de los minutos
# 10 a 70 (los primeros 10 minutos incluyen la lectura inicial del historial).
#
# Mientras corre, el equipo no se suspende (la pantalla sí puede apagarse).
#
# La app solo LEE ~/.claude. Escribe en %APPDATA%\dev-cockpit (su base de datos y,
# si la hubiera, panics.log). Este guion escribe solo en -OutDir.
#
# Uso:
#   powershell -ExecutionPolicy Bypass -File b9-soak.ps1 [-Hours 24] [-App <ruta del .exe>] [-OutDir <carpeta>]
#   powershell -ExecutionPolicy Bypass -File b9-soak.ps1 -Summarize -OutDir <carpeta con muestras.csv>
param(
  [double]$Hours = 24,
  [string]$App = (Join-Path $env:LOCALAPPDATA 'DEV COCKPIT\dev-cockpit.exe'),
  [string]$OutDir = (Join-Path ([Environment]::GetFolderPath('Desktop')) 'dev-cockpit-b9'),
  [string]$UserDataMarker = 'dev-cockpit\EBWebView',
  [switch]$Summarize
)

$ErrorActionPreference = 'Stop'
$csvPath = Join-Path $OutDir 'muestras.csv'
$eventsPath = Join-Path $OutDir 'eventos.log'
$summaryPath = Join-Path $OutDir 'resumen.txt'
$panicsPath = Join-Path $env:APPDATA 'dev-cockpit\panics.log'

function Write-Summary {
  $rows = @(Import-Csv $csvPath)
  $culture = [Globalization.CultureInfo]::InvariantCulture
  $num = { param($v) if ($v -eq '' -or $null -eq $v) { $null } else { [double]::Parse($v, $culture) } }
  $valid = @($rows | Where-Object { $_.valid -eq 'True' })
  $lines = @()
  $lines += "muestras: $($rows.Count)  validas (metodo 1 = metodo 2): $($valid.Count)  invalidas: $($rows.Count - $valid.Count)"
  if ($rows.Count -gt 0) {
    $hours = (& $num $rows[-1].elapsed_s) / 3600
    $lines += ('duracion: {0:N2} h' -f $hours)
  }
  if ($valid.Count -gt 0) {
    $total = $valid | ForEach-Object { & $num $_.total_mb }
    $m = $total | Measure-Object -Average -Maximum -Minimum
    $lines += ('memoria privada total (MB): media {0:N1}  min {1:N1}  max {2:N1}' -f $m.Average, $m.Minimum, $m.Maximum)
    $hostMb = $valid | ForEach-Object { & $num $_.host_mb } | Measure-Object -Average -Maximum
    $lines += ('  proceso Rust (MB): media {0:N1}  max {1:N1}  (objetivo < 60)' -f $hostMb.Average, $hostMb.Maximum)
    $base = @($valid | Where-Object { $e = & $num $_.elapsed_s; $e -ge 600 -and $e -lt 4200 } | ForEach-Object { & $num $_.total_mb })
    $endAt = (& $num $valid[-1].elapsed_s) - 3600
    $last = @($valid | Where-Object { (& $num $_.elapsed_s) -ge $endAt } | ForEach-Object { & $num $_.total_mb })
    if ($base.Count -gt 0 -and $last.Count -gt 0 -and $endAt -ge 4200) {
      $b = ($base | Measure-Object -Average).Average
      $l = ($last | Measure-Object -Average).Average
      $lines += ('crecimiento de RAM: {0:N1} MB (min. 10-70) -> {1:N1} MB (ultima hora) = {2:N1} %  (objetivo < 10 %)' -f $b, $l, (100 * ($l - $b) / $b))
    } else {
      $lines += 'crecimiento de RAM: la prueba es demasiado corta para compararlo (hace falta mas de 2 h)'
    }
    $cpu = @($valid | Where-Object { $_.cpu_pct -ne '' } | ForEach-Object { & $num $_.cpu_pct }) | Measure-Object -Average -Maximum
    $lines += ('CPU del grupo (% del equipo): media {0:N2}  max {1:N2}' -f $cpu.Average, $cpu.Maximum)
    $handles = $valid | ForEach-Object { & $num $_.host_handles } | Measure-Object -Minimum -Maximum
    $lines += ('handles del proceso Rust: min {0}  max {1}' -f $handles.Minimum, $handles.Maximum)
  }
  $exits = if (Test-Path $eventsPath) { @(Get-Content $eventsPath | Where-Object { $_ -like '*app_exit*' }).Count } else { 0 }
  $panics = if (Test-Path (Join-Path $OutDir 'panics.log')) { @(Get-Content (Join-Path $OutDir 'panics.log')).Count } else { 0 }
  $lines += "salidas inesperadas de la app: $exits   panicos registrados: $panics  (objetivo 0)"
  $lines | Set-Content -Encoding UTF8 $summaryPath
  $lines
}

if ($Summarize) { Write-Summary; return }

if (-not (Test-Path $App)) { throw "No se encuentra la app en '$App'. Indica la ruta con -App." }
if (Get-Process dev-cockpit -ErrorAction SilentlyContinue) {
  throw 'DEV COCKPIT ya esta abierta. Cierrala (tambien desde la bandeja) y vuelve a lanzar la prueba.'
}
New-Item -ItemType Directory -Force $OutDir | Out-Null
if (Test-Path $csvPath) { throw "Ya hay una prueba en '$OutDir'. Usa otra carpeta con -OutDir." }

# Equipo y versiones: sin nombres de usuario ni rutas.
$os = Get-CimInstance Win32_OperatingSystem
$cpuInfo = Get-CimInstance Win32_Processor | Select-Object -First 1
$wv = $null
foreach ($key in @('HKLM:\SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}',
                   'HKCU:\SOFTWARE\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}')) {
  if (-not $wv) { $wv = (Get-ItemProperty $key -ErrorAction SilentlyContinue).pv }
}
$appFile = Get-Item $App
$panicsBefore = if (Test-Path $panicsPath) { @(Get-Content $panicsPath).Count } else { 0 }
[ordered]@{
  inicio = (Get-Date).ToString('o')
  horas = $Hours
  windows = "$($os.Caption) $($os.Version)"
  cpu = $cpuInfo.Name.Trim()
  nucleos_logicos = [Environment]::ProcessorCount
  ram_gb = [math]::Round($os.TotalVisibleMemorySize / 1MB, 1)
  webview2 = $wv
  app_version = $appFile.VersionInfo.ProductVersion
  app_bytes = $appFile.Length
} | ConvertTo-Json | Set-Content -Encoding UTF8 (Join-Path $OutDir 'equipo.json')

# Que el equipo no se suspenda mientras dura la prueba (la pantalla sí puede
# apagarse). Windows lo anula solo en cuanto este proceso termina.
Add-Type -Namespace B9 -Name Power -MemberDefinition '[DllImport("kernel32.dll")] public static extern uint SetThreadExecutionState(uint flags);'
[void][B9.Power]::SetThreadExecutionState([uint32]'0x80000001') # ES_CONTINUOUS | ES_SYSTEM_REQUIRED

$procsLog = Join-Path $OutDir 'procs.json'
$env:DEV_COCKPIT_PROCS_LOG = $procsLog
$proc = Start-Process -FilePath $App -PassThru
"$((Get-Date).ToString('o'))`tapp_start`tpid=$($proc.Id)" | Add-Content -Encoding UTF8 $eventsPath

$cores = [Environment]::ProcessorCount
$culture = [Globalization.CultureInfo]::InvariantCulture
$writer = New-Object IO.StreamWriter($csvPath, $false, (New-Object Text.UTF8Encoding($false)))
$writer.AutoFlush = $true
$writer.WriteLine('ts,elapsed_s,valid,count1,count2,cpu_pct,total_mb,working_mb,host_mb,browser_mb,renderer_mb,gpu_mb,utility_mb,crashpad_mb,host_handles,host_threads')
$start = Get-Date
$end = $start.AddHours($Hours)
$prevCpu = @{}; $prevTime = $null; $nextReport = $start.AddHours(1)
$fmt = { param($v) if ($null -eq $v) { '' } else { ([double]$v).ToString('0.###', $culture) } }

try {
  while ((Get-Date) -lt $end) {
    Start-Sleep -Milliseconds 1000
    $now = Get-Date
    $proc.Refresh()
    if ($proc.HasExited) {
      "$($now.ToString('o'))`tapp_exit`tcode=$($proc.ExitCode)" | Add-Content -Encoding UTF8 $eventsPath
      Write-Warning "La app se ha cerrado (codigo $($proc.ExitCode)). Fin de la prueba."
      break
    }

    # Método 1
    $report = $null
    try { $report = Get-Content $procsLog -Raw -ErrorAction Stop | ConvertFrom-Json } catch {}
    $kinds = @{}
    $set1 = @()
    if ($report -and $report.ok) {
      foreach ($p in $report.processes) { $kinds[[int]$p.pid] = $p.kind }
      $set1 = @($report.processes | Where-Object { $_.kind -ne 'host' } | ForEach-Object { [int]$_.pid } | Sort-Object)
    }

    # Método 2
    $all = @(Get-CimInstance Win32_Process -Filter "Name='msedgewebview2.exe'")
    $parent = @{}
    foreach ($p in $all) { $parent[[int]$p.ProcessId] = [int]$p.ParentProcessId }
    $mine = @($all | Where-Object {
        $id = [int]$_.ProcessId; $hops = 0
        while ($parent.ContainsKey($id) -and $hops -lt 8) { $id = $parent[$id]; $hops++ }
        ($id -eq $proc.Id) -and ($_.CommandLine -like "*$UserDataMarker*")
      })
    $crashpad = @($mine | Where-Object { $_.CommandLine -like '*--type=crashpad-handler*' } | ForEach-Object { [int]$_.ProcessId })
    foreach ($id in $crashpad) { $kinds[$id] = 'crashpad' }
    $set2 = @($mine | ForEach-Object { [int]$_.ProcessId } | Where-Object { $crashpad -notcontains $_ } | Sort-Object)
    $valid = ($set1.Count -gt 0) -and (($set1 -join ',') -eq ($set2 -join ','))
    $kinds[$proc.Id] = 'host'

    $private = @{}; $working = 0; $cpuNow = @{}; $handles = $null; $threads = $null
    foreach ($id in @($proc.Id) + $set1 + $crashpad) {
      $p = Get-Process -Id $id -ErrorAction SilentlyContinue
      if (-not $p) { continue }
      $kind = $kinds[$id]
      $private[$kind] = [double]$private[$kind] + $p.PrivateMemorySize64
      $working += $p.WorkingSet64
      $cpuNow[$id] = $p.TotalProcessorTime.TotalMilliseconds
      if ($id -eq $proc.Id) { $handles = $p.HandleCount; $threads = $p.Threads.Count }
    }
    $cpuPct = $null
    if ($prevTime) {
      $elapsed = ($now - $prevTime).TotalMilliseconds
      $used = 0
      foreach ($id in $cpuNow.Keys) { if ($prevCpu.ContainsKey($id)) { $used += $cpuNow[$id] - $prevCpu[$id] } }
      $cpuPct = 100 * $used / ($elapsed * $cores)
    }
    $prevCpu = $cpuNow; $prevTime = $now

    $total = ($private.Values | Measure-Object -Sum).Sum
    $mb = { param($k) & $fmt ([double]$private[$k] / 1MB) }
    $writer.WriteLine(($now.ToString('o'), (& $fmt ($now - $start).TotalSeconds), $valid, $set1.Count, $set2.Count,
        (& $fmt $cpuPct), (& $fmt ($total / 1MB)), (& $fmt ($working / 1MB)), (& $mb 'host'), (& $mb 'browser'),
        (& $mb 'renderer'), (& $mb 'gpu'), (& $mb 'utility'), (& $mb 'crashpad'), $handles, $threads) -join ',')

    if ($now -ge $nextReport) {
      Write-Host ('{0:HH:mm} · {1:N1} h · {2:N1} MB privados · CPU {3:N2} %' -f $now, ($now - $start).TotalHours, ($total / 1MB), $cpuPct)
      $nextReport = $nextReport.AddHours(1)
    }
  }
} finally {
  $writer.Dispose()
  # Pánicos nuevos durante la prueba (solo hora, hilo y fichero:línea del código).
  if (Test-Path $panicsPath) {
    $new = @(Get-Content $panicsPath | Select-Object -Skip $panicsBefore)
    if ($new.Count -gt 0) { $new | Set-Content -Encoding UTF8 (Join-Path $OutDir 'panics.log') }
  }
  "$((Get-Date).ToString('o'))`tfin" | Add-Content -Encoding UTF8 $eventsPath
}

Write-Summary
"Resultados en $OutDir. La app sigue abierta."
