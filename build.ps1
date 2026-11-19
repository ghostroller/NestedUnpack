[CmdletBinding()]
param(
    [string]$OutputDirectory = 'bin',
    [string]$Cxx = 'clang++'
)

$ErrorActionPreference = 'Stop'
if ($env:OS -ne 'Windows_NT') { throw 'NestedUnpack builds on Windows.' }

$go = (Get-Command go -CommandType Application -ErrorAction Stop).Source
$compiler = (Get-Command $Cxx -CommandType Application -ErrorAction Stop).Source
if (-not [IO.Path]::IsPathRooted($OutputDirectory)) {
    $OutputDirectory = Join-Path $PSScriptRoot $OutputDirectory
}
$output = [IO.Path]::GetFullPath($OutputDirectory)
[IO.Directory]::CreateDirectory($output) | Out-Null

if (-not ('NestedUnpack.BuildResources' -as [type])) {
    Add-Type -Namespace NestedUnpack -Name BuildResources -MemberDefinition @'
        [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
        private static extern IntPtr BeginUpdateResource(string fileName, bool deleteExisting);

        [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
        private static extern bool UpdateResource(IntPtr update, IntPtr type, IntPtr name,
                                                  ushort language, byte[] data, uint size);

        [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
        private static extern bool EndUpdateResource(IntPtr update, bool discard);

        public static void EmbedManifest(string fileName, byte[] manifest) {
            IntPtr update = BeginUpdateResource(fileName, false);
            if (update == IntPtr.Zero) {
                throw new System.ComponentModel.Win32Exception(Marshal.GetLastWin32Error());
            }
            // RT_MANIFEST, CREATEPROCESS_MANIFEST_RESOURCE_ID, LANG_NEUTRAL.
            if (!UpdateResource(update, (IntPtr)24, (IntPtr)1, 0, manifest, (uint)manifest.Length)) {
                int error = Marshal.GetLastWin32Error();
                EndUpdateResource(update, true);
                throw new System.ComponentModel.Win32Exception(error);
            }
            if (!EndUpdateResource(update, false)) {
                throw new System.ComponentModel.Win32Exception(Marshal.GetLastWin32Error());
            }
        }
'@
}

$stage = Join-Path $output ('.build-' + [Guid]::NewGuid().ToString('N'))
[IO.Directory]::CreateDirectory($stage) | Out-Null
$extractor = Join-Path $stage 'NestedUnpack.exe'
$gui = Join-Path $stage 'NestedUnpackGUI.exe'
$helper = Join-Path $stage 'NestedUnpackShell.exe'
$object = Join-Path $stage 'shell.o'
$shell = Join-Path $PSScriptRoot 'shell'
$source = Join-Path $shell 'shell.cpp'

Push-Location -LiteralPath $PSScriptRoot
try {
    & $go build -trimpath -ldflags '-s -w' -o $extractor .
    if ($LASTEXITCODE -ne 0) { throw "Go build failed with exit code $LASTEXITCODE." }

    & $go build -trimpath -ldflags '-s -w -H=windowsgui' -o $gui .
    if ($LASTEXITCODE -ne 0) { throw "Go GUI build failed with exit code $LASTEXITCODE." }
    [NestedUnpack.BuildResources]::EmbedManifest($gui, [IO.File]::ReadAllBytes((Join-Path $shell 'gui.manifest')))

    & $compiler -std=c++17 -O2 -Wall -Wextra -Wpedantic -c $source -o $object
    if ($LASTEXITCODE -ne 0) { throw "Shell helper compilation failed with exit code $LASTEXITCODE." }

    & $compiler -municode -mwindows -static -s $object -lshell32 -lole32 -luuid -o $helper
    if ($LASTEXITCODE -ne 0) { throw "Shell helper linking failed with exit code $LASTEXITCODE." }

    foreach ($artifact in @($extractor, $gui, $helper)) {
        $destination = Join-Path $output ([IO.Path]::GetFileName($artifact))
        [IO.File]::Copy($artifact, $destination, $true)
        $destination
    }
} finally {
    Pop-Location
    foreach ($temporary in @($extractor, $gui, $helper, $object)) {
        [IO.File]::Delete($temporary)
    }
    [IO.Directory]::Delete($stage, $false)
}
