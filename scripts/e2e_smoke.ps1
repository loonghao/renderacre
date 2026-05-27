$ErrorActionPreference = "Stop"

$root = Split-Path -Parent $PSScriptRoot
$port = if ($env:RENDERACRE_E2E_PORT) { [int]$env:RENDERACRE_E2E_PORT } else { 17878 }
$controllerOut = Join-Path $root "target\controller-e2e.out.log"
$controllerErr = Join-Path $root "target\controller-e2e.err.log"
$workerOut = Join-Path $root "target\worker-e2e.out.log"
$workerErr = Join-Path $root "target\worker-e2e.err.log"
$controller = $null
$worker = $null
$pushedLocation = $false

function Wait-Controller {
    for ($i = 0; $i -lt 60; $i++) {
        try {
            $health = Invoke-RestMethod -Uri "http://127.0.0.1:$port/healthz" -TimeoutSec 1
            if ($health.status -eq "ok") { return }
        } catch {
            Start-Sleep -Milliseconds 250
        }
    }
    throw "controller did not become ready on port $port"
}

function Wait-Job([string]$JobId) {
    for ($i = 0; $i -lt 80; $i++) {
        $job = Invoke-RestMethod -Uri "http://127.0.0.1:$port/v1/jobs/$JobId" -TimeoutSec 2
        if ($job.state -eq "succeeded" -or $job.state -eq "failed") { return $job }
        Start-Sleep -Milliseconds 500
    }
    throw "job $JobId did not finish"
}

try {
    Push-Location $root
    $pushedLocation = $true
    cargo build -p renderacre-controller -p renderacre-worker

    $controllerExe = Join-Path $root "target\debug\renderacre-controller.exe"
    $workerExe = Join-Path $root "target\debug\renderacre-worker.exe"
    if (-not (Test-Path $controllerExe)) { $controllerExe = Join-Path $root "target\debug\renderacre-controller" }
    if (-not (Test-Path $workerExe)) { $workerExe = Join-Path $root "target\debug\renderacre-worker" }

    $controllerArgs = @{
        FilePath = $controllerExe
        ArgumentList = @("--bind", "127.0.0.1:$port")
        PassThru = $true
        RedirectStandardOutput = $controllerOut
        RedirectStandardError = $controllerErr
    }
    if ($IsWindows) { $controllerArgs.WindowStyle = "Hidden" }
    $controller = Start-Process @controllerArgs
    Wait-Controller

    $workerArgs = @{
        FilePath = $workerExe
        ArgumentList = @("--controller", "http://127.0.0.1:$port", "--name", "e2e-worker")
        PassThru = $true
        RedirectStandardOutput = $workerOut
        RedirectStandardError = $workerErr
    }
    if ($IsWindows) { $workerArgs.WindowStyle = "Hidden" }
    $worker = Start-Process @workerArgs
    Start-Sleep -Seconds 1

    $directJob = Invoke-RestMethod -Method Post -Uri "http://127.0.0.1:$port/v1/jobs" -ContentType "application/json" -InFile ".\examples\hello_job.json"
    $directFinal = Wait-Job $directJob.id
    if ($directFinal.state -ne "succeeded") { throw "direct job ended as $($directFinal.state)" }

    $template = @"
specificationVersion: jobtemplate-2023-09
name: PythonFrames
steps:
  - name: Frame
    parameterSpace:
      taskParameterDefinitions:
        - name: Frame
          type: INT
          range: "1-2"
    script:
      actions:
        onRun:
          command: python
          args:
            - -c
            - "print('openjd frame {{ Task.Param.Frame }}')"
"@
    $openjdPayload = @{
        name = "openjd-python-frames"
        openjd = @{
            template_yaml = $template
            parameters = @{}
        }
    } | ConvertTo-Json -Depth 20
    $openjdJob = Invoke-RestMethod -Method Post -Uri "http://127.0.0.1:$port/v1/jobs" -ContentType "application/json" -Body $openjdPayload
    $openjdFinal = Wait-Job $openjdJob.id
    if ($openjdFinal.state -ne "succeeded") { throw "openjd job ended as $($openjdFinal.state)" }

    Write-Host "E2E passed: direct=$($directFinal.id) openjd=$($openjdFinal.id)"
} finally {
    if ($worker -and -not $worker.HasExited) { Stop-Process -Id $worker.Id -Force }
    if ($controller -and -not $controller.HasExited) { Stop-Process -Id $controller.Id -Force }
    if ($pushedLocation) { Pop-Location }
}
