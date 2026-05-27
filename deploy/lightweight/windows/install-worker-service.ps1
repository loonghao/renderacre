param(
    [string]$ServiceName = "RenderacreWorker",
    [string]$Executable = "C:\Program Files\Renderacre\renderacre-worker.exe",
    [string]$Config = "C:\ProgramData\Renderacre\worker.yaml"
)

$ErrorActionPreference = "Stop"

if (-not (Test-Path -LiteralPath $Executable)) {
    throw "Worker executable not found: $Executable"
}
if (-not (Test-Path -LiteralPath $Config)) {
    throw "Worker config not found: $Config"
}

$binaryPath = "`"$Executable`" --config `"$Config`""

if (Get-Service -Name $ServiceName -ErrorAction SilentlyContinue) {
    sc.exe config $ServiceName binPath= $binaryPath | Out-Null
} else {
    New-Service `
        -Name $ServiceName `
        -DisplayName "Renderacre Worker" `
        -Description "Renderacre render worker" `
        -BinaryPathName $binaryPath `
        -StartupType Automatic | Out-Null
}

Write-Host "Installed $ServiceName. Start it with: Start-Service $ServiceName"
