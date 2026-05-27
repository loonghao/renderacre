param(
    [string]$ServiceName = "RenderacreController",
    [string]$Executable = "C:\Program Files\Renderacre\renderacre-controller.exe",
    [string]$Config = "C:\ProgramData\Renderacre\controller.yaml"
)

$ErrorActionPreference = "Stop"

if (-not (Test-Path -LiteralPath $Executable)) {
    throw "Controller executable not found: $Executable"
}
if (-not (Test-Path -LiteralPath $Config)) {
    throw "Controller config not found: $Config"
}

$binaryPath = "`"$Executable`" --config `"$Config`""

if (Get-Service -Name $ServiceName -ErrorAction SilentlyContinue) {
    sc.exe config $ServiceName binPath= $binaryPath | Out-Null
} else {
    New-Service `
        -Name $ServiceName `
        -DisplayName "Renderacre Controller" `
        -Description "Renderacre durable controller and dashboard" `
        -BinaryPathName $binaryPath `
        -StartupType Automatic | Out-Null
}

Write-Host "Installed $ServiceName. Start it with: Start-Service $ServiceName"
