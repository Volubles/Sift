param(
    [Parameter(Mandatory)][string]$Executable,
    [Parameter(Mandatory)][string]$Database,
    [Parameter(Mandatory)][string]$Output,
    [int]$Seconds = 60,
    [ValidateSet('default','dx12','vulkan','opengl')][string]$Backend = 'default',
    [ValidateSet('wgpu','glow')][string]$Renderer = 'wgpu'
)

$ErrorActionPreference = 'Stop'
$executableFile = Get-Item -LiteralPath $Executable
$databaseFile = Get-Item -LiteralPath $Database
$outputPath = [System.IO.Path]::GetFullPath($Output)
if ($Seconds -lt 1 -or $Seconds -gt 60) { throw 'Measurement duration must be 1-60 seconds.' }
if (Test-Path -LiteralPath $outputPath) { throw 'Refusing to overwrite measurement output.' }
New-Item -ItemType Directory -Path ([System.IO.Path]::GetDirectoryName($outputPath)) -Force | Out-Null
$timer = [System.Diagnostics.Stopwatch]::StartNew()
$processOptions = @{ FilePath=$executableFile.FullName; ArgumentList=@('--database', ('"' + $databaseFile.FullName + '"'), 'browse'); WindowStyle='Hidden'; PassThru=$true }
$processOptions.Environment = @{ SIFT_RENDERER=$Renderer }
if ($Backend -ne 'default') { $processOptions.Environment.WGPU_BACKEND=$Backend }
$measuredApp = Start-Process @processOptions
try {
    while ($measuredApp.MainWindowHandle -eq [IntPtr]::Zero -and $timer.Elapsed.TotalSeconds -lt 15) {
        if ($measuredApp.HasExited) { throw 'Application exited before opening its window.' }
        Start-Sleep -Milliseconds 20
        $measuredApp.Refresh()
    }
    if ($measuredApp.MainWindowHandle -eq [IntPtr]::Zero) { throw 'Native window did not appear.' }
    $windowSeconds = $timer.Elapsed.TotalSeconds
    Start-Sleep -Seconds 5
    $measuredApp.Refresh()
    $cpuStart = $measuredApp.TotalProcessorTime.TotalSeconds
    $measurement = [System.Diagnostics.Stopwatch]::StartNew()
    $samples = @()
    while ($measurement.Elapsed.TotalSeconds -lt $Seconds) {
        Start-Sleep -Milliseconds 2000
        $measuredApp.Refresh()
        if ($measuredApp.HasExited) { throw 'Application exited during measurement.' }
        $samples += [pscustomobject]@{ Seconds=$measurement.Elapsed.TotalSeconds; PrivateBytes=$measuredApp.PrivateMemorySize64; WorkingSetBytes=$measuredApp.WorkingSet64; CpuSeconds=$measuredApp.TotalProcessorTime.TotalSeconds }
    }
    $elapsed = $measurement.Elapsed.TotalSeconds
    $cpuDelta = $measuredApp.TotalProcessorTime.TotalSeconds - $cpuStart
    $result = [ordered]@{
        Commit = (git rev-parse HEAD)
        WorkingTreeDirty = [bool]@(git status --porcelain).Count
        Profile = 'release'
        Renderer = $Renderer
        BackendOverride = $Backend
        LogicalProcessors = [Environment]::ProcessorCount
        TimeToNativeWindowSeconds = $windowSeconds
        MeasurementSeconds = $elapsed
        CpuPercentOfMachine = 100 * $cpuDelta / $elapsed / [Environment]::ProcessorCount
        MeanPrivateMiB = ($samples | Measure-Object PrivateBytes -Average).Average / 1MB
        PeakPrivateMiB = ($samples | Measure-Object PrivateBytes -Maximum).Maximum / 1MB
        MeanWorkingSetMiB = ($samples | Measure-Object WorkingSetBytes -Average).Average / 1MB
        GpuMemory = 'Not measured'
        Limitations = 'Synthetic metadata; window appearance is not interactive readiness; no input or audio activity.'
        Samples = $samples
    }
    [System.IO.File]::WriteAllText($outputPath, ($result | ConvertTo-Json -Depth 4) + "`n", [System.Text.UTF8Encoding]::new($false))
    $result.Remove('Samples')
    $result | ConvertTo-Json
} finally {
    if (-not $measuredApp.HasExited) {
        if (-not $measuredApp.CloseMainWindow()) { throw 'Could not request closure of the test application.' }
        if (-not $measuredApp.WaitForExit(5000)) { throw 'Test application did not exit after its close request.' }
    }
}
