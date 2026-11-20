[CmdletBinding()]
param(
    [string]$OutputDirectory = 'bin'
)

$ErrorActionPreference = 'Stop'
if ($env:OS -ne 'Windows_NT') { throw 'Use cargo build --release --locked on this platform.' }

$cargo = (Get-Command cargo -CommandType Application -ErrorAction Stop | Select-Object -First 1).Source
if (-not [IO.Path]::IsPathRooted($OutputDirectory)) {
    $OutputDirectory = Join-Path $PSScriptRoot $OutputDirectory
}
$output = [IO.Path]::GetFullPath($OutputDirectory)
$target = 'x86_64-pc-windows-msvc'

Push-Location -LiteralPath $PSScriptRoot
try {
    $artifact = & $cargo build --release --locked --target $target --message-format=json-render-diagnostics |
        Where-Object { $_.StartsWith('{"reason":"compiler-artifact"') } |
        ForEach-Object { ConvertFrom-Json -InputObject $_ } |
        Where-Object { $_.target.name -eq 'NestedUnpack' -and $_.executable }
    if ($LASTEXITCODE -ne 0) { throw "Cargo build failed with exit code $LASTEXITCODE." }
    if (-not $artifact.executable) { throw 'Cargo did not return a NestedUnpack executable.' }

    [IO.Directory]::CreateDirectory($output) | Out-Null
    $destination = Join-Path $output 'NestedUnpack.exe'
    if ($artifact.executable -ne $destination) {
        [IO.File]::Copy($artifact.executable, $destination, $true)
    }
    $destination
} finally {
    Pop-Location
}
