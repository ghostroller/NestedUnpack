[CmdletBinding()]
param(
    [string]$OutputDirectory = 'bin',
    [string]$Cxx = 'clang++'
)

$ErrorActionPreference = 'Stop'
if ($env:OS -ne 'Windows_NT') { throw 'OuterRAR builds on Windows.' }

$go = (Get-Command go -CommandType Application -ErrorAction Stop).Source
$compiler = (Get-Command $Cxx -CommandType Application -ErrorAction Stop).Source
if (-not [IO.Path]::IsPathRooted($OutputDirectory)) {
    $OutputDirectory = Join-Path $PSScriptRoot $OutputDirectory
}
$output = [IO.Path]::GetFullPath($OutputDirectory)
[IO.Directory]::CreateDirectory($output) | Out-Null

$stage = Join-Path $output ('.build-' + [Guid]::NewGuid().ToString('N'))
[IO.Directory]::CreateDirectory($stage) | Out-Null
$extractor = Join-Path $stage 'OuterRar.exe'
$helper = Join-Path $stage 'OuterRarShell.exe'
$object = Join-Path $stage 'shell.o'
$source = Join-Path (Join-Path $PSScriptRoot 'shell') 'shell.cpp'

Push-Location -LiteralPath $PSScriptRoot
try {
    & $go build -trimpath -ldflags '-s -w' -o $extractor .
    if ($LASTEXITCODE -ne 0) { throw "Go build failed with exit code $LASTEXITCODE." }

    & $compiler -std=c++17 -O2 -Wall -Wextra -Wpedantic -c $source -o $object
    if ($LASTEXITCODE -ne 0) { throw "Shell helper compilation failed with exit code $LASTEXITCODE." }

    & $compiler -municode -mwindows -static -s $object -lshell32 -lole32 -luuid -o $helper
    if ($LASTEXITCODE -ne 0) { throw "Shell helper linking failed with exit code $LASTEXITCODE." }

    foreach ($artifact in @($extractor, $helper)) {
        $destination = Join-Path $output ([IO.Path]::GetFileName($artifact))
        [IO.File]::Copy($artifact, $destination, $true)
        $destination
    }
} finally {
    Pop-Location
    foreach ($temporary in @($extractor, $helper, $object)) {
        [IO.File]::Delete($temporary)
    }
    [IO.Directory]::Delete($stage, $false)
}
