# B8: muestrea CPU y memoria SOLO de los procesos de DEV COCKPIT (C2, SPEC §11.4).
#
# Dos métodos para identificar el grupo WebView2 propio, que deben coincidir:
#   1. GetProcessInfos del entorno WebView2 de la app (fichero DEV_COCKPIT_PROCS_LOG).
#   2. Árbol de procesos: msedgewebview2.exe descendientes de dev-cockpit.exe cuyo
#      CommandLine contiene el --user-data-dir propio.
# Una muestra en la que los conjuntos no coinciden se marca como inválida.
#
# GetProcessInfos no incluye el proceso `crashpad-handler` del grupo (observado con
# el runtime 157). Es nuestro (mismo --user-data-dir), así que se suma a la memoria
# como tipo `crashpad`, pero queda fuera de la comparación entre métodos.
#
# Uso: sample-procs.ps1 -ProcsLog <fichero> -Seconds 60 [-UserDataMarker 'dev-cockpit\EBWebView']
param(
  [Parameter(Mandatory)] [string]$ProcsLog,
  [int]$Seconds = 60,
  [string]$UserDataMarker = 'dev-cockpit\EBWebView'
)

$cores = [Environment]::ProcessorCount
$samples = @()
$prevCpu = @{}
$prevTime = $null

for ($i = 0; $i -lt $Seconds; $i++) {
  Start-Sleep -Milliseconds 1000
  $hostProc = Get-Process dev-cockpit -ErrorAction SilentlyContinue | Select-Object -First 1
  if (-not $hostProc) { continue }

  # Método 1
  $report = $null
  try { $report = Get-Content $ProcsLog -Raw -ErrorAction Stop | ConvertFrom-Json } catch {}
  if (-not $report -or -not $report.ok) { continue }
  $kinds = @{}
  foreach ($p in $report.processes) { $kinds[[int]$p.pid] = $p.kind }
  $set1 = @($report.processes | Where-Object { $_.kind -ne 'host' } | ForEach-Object { [int]$_.pid } | Sort-Object)

  # Método 2
  $all = Get-CimInstance Win32_Process -Filter "Name='msedgewebview2.exe'"
  $parent = @{}
  foreach ($p in $all) { $parent[[int]$p.ProcessId] = [int]$p.ParentProcessId }
  $mine = @($all | Where-Object {
      $id = [int]$_.ProcessId; $hops = 0
      while ($parent.ContainsKey($id) -and $hops -lt 8) { $id = $parent[$id]; $hops++ }
      ($id -eq $hostProc.Id) -and ($_.CommandLine -like "*$UserDataMarker*")
    })
  $crashpad = @($mine | Where-Object { $_.CommandLine -like "*--type=crashpad-handler*" } | ForEach-Object { [int]$_.ProcessId })
  foreach ($id in $crashpad) { $kinds[$id] = "crashpad" }
  $set2 = @($mine | ForEach-Object { [int]$_.ProcessId } | Where-Object { $crashpad -notcontains $_ } | Sort-Object)

  $valid = (($set1 -join ',') -eq ($set2 -join ','))

  $now = Get-Date
  $private = @{}; $working = 0; $cpuNow = @{}
  foreach ($id in @($hostProc.Id) + $set1 + $crashpad) {
    $p = Get-Process -Id $id -ErrorAction SilentlyContinue
    if (-not $p) { continue }
    $kind = $kinds[$id]
    $private[$kind] = [double]$private[$kind] + $p.PrivateMemorySize64
    $working += $p.WorkingSet64
    $cpuNow[$id] = $p.TotalProcessorTime.TotalMilliseconds
  }
  $cpuPct = $null
  if ($prevTime) {
    $elapsed = ($now - $prevTime).TotalMilliseconds
    $used = 0
    foreach ($id in $cpuNow.Keys) { if ($prevCpu.ContainsKey($id)) { $used += $cpuNow[$id] - $prevCpu[$id] } }
    $cpuPct = 100 * $used / ($elapsed * $cores)
  }
  $prevCpu = $cpuNow; $prevTime = $now

  $samples += [pscustomobject]@{
    Valid = $valid; Count1 = $set1.Count; Count2 = $set2.Count; Cpu = $cpuPct
    Total = ($private.Values | Measure-Object -Sum).Sum; Working = $working
    HostMB = $private['host'] / 1MB; BrowserMB = $private['browser'] / 1MB
    RendererMB = $private['renderer'] / 1MB; GpuMB = $private['gpu'] / 1MB; UtilityMB = $private['utility'] / 1MB
    CrashpadMB = $private['crashpad'] / 1MB
  }
}

$ok = @($samples | Where-Object Valid)
"muestras: $($samples.Count)  válidas (método 1 = método 2): $($ok.Count)  núcleos lógicos: $cores"
if ($samples.Count -gt 0) { "procesos WebView2 propios (sin crashpad): método 1 = $($samples[-1].Count1), método 2 = $($samples[-1].Count2)" }
if ($ok.Count -gt 0) {
  $fmt = { param($prop) $m = $ok | Measure-Object -Property $prop -Average -Maximum; '{0,7:N1} media  {1,7:N1} máx' -f $m.Average, $m.Maximum }
  $total = $ok | ForEach-Object { $_.Total / 1MB } | Measure-Object -Average -Maximum
  $ws = $ok | ForEach-Object { $_.Working / 1MB } | Measure-Object -Average -Maximum
  'private bytes total (MB): {0,7:N1} media  {1,7:N1} máx' -f $total.Average, $total.Maximum
  "  proceso Rust (MB):      $(& $fmt HostMB)"
  "  browser (MB):           $(& $fmt BrowserMB)"
  "  renderer (MB):          $(& $fmt RendererMB)"
  "  gpu (MB):               $(& $fmt GpuMB)"
  "  utility (MB):           $(& $fmt UtilityMB)"
  "  crashpad (MB):          $(& $fmt CrashpadMB)"
  'working set total (MB):   {0,7:N1} media  {1,7:N1} máx  (referencia)' -f $ws.Average, $ws.Maximum
  $cpu = $ok | Where-Object { $null -ne $_.Cpu } | Measure-Object -Property Cpu -Average -Maximum
  'CPU del grupo (% del total del equipo): {0:N2} media  {1:N2} máx' -f $cpu.Average, $cpu.Maximum
}
$wv = (Get-ItemProperty 'HKLM:\SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}' -ErrorAction SilentlyContinue).pv
"runtime WebView2: $wv"
