# GPU + private memory of the Playwright-launched Edge (identified by its temp profile dir).
# Uses the Windows "GPU Process Memory" perf counters (WDDM, Windows 10+).
$procs = @(Get-CimInstance Win32_Process -Filter "Name='msedge.exe'" | Where-Object { $_.CommandLine -like '*playwright_chromiumdev_profile*' })
$ids = @($procs | ForEach-Object { [int]$_.ProcessId })
$gpuPid = ($procs | Where-Object { $_.CommandLine -like '*--type=gpu-process*' } | Select-Object -First 1).ProcessId
$rendPids = @($procs | Where-Object { $_.CommandLine -like '*--type=renderer*' } | ForEach-Object { [int]$_.ProcessId })
$ctr = Get-Counter '\GPU Process Memory(*)\Dedicated Usage', '\GPU Process Memory(*)\Shared Usage' -ErrorAction SilentlyContinue
$ded = 0; $sh = 0
foreach ($s in $ctr.CounterSamples) {
  if ($s.InstanceName -match '^pid_(\d+)_') {
    if ($ids -contains [int]$matches[1]) { if ($s.Path -like '*dedicated usage') { $ded += $s.CookedValue } else { $sh += $s.CookedValue } }
  }
}
$priv = 0; foreach ($i in $ids) { $p = Get-Process -Id $i -ErrorAction SilentlyContinue; if ($p) { $priv += $p.PrivateMemorySize64 } }
$rend = 0; foreach ($i in $rendPids) { $p = Get-Process -Id $i -ErrorAction SilentlyContinue; if ($p) { $rend += $p.PrivateMemorySize64 } }
@{ gpuDedicatedMB = [math]::Round($ded / 1MB); gpuSharedMB = [math]::Round($sh / 1MB); privateMB = [math]::Round($priv / 1MB); rendererPrivateMB = [math]::Round($rend / 1MB); procs = $ids.Count } | ConvertTo-Json -Compress
