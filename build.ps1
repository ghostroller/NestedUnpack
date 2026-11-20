[CmdletBinding()]
param(
    [string]$OutputDirectory = 'bin'
)

$ErrorActionPreference = 'Stop'
if ($env:OS -ne 'Windows_NT') { throw 'NestedUnpack builds on Windows.' }

$go = (Get-Command go -CommandType Application -ErrorAction Stop).Source
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
$manifest = Join-Path $PSScriptRoot 'app.manifest'
$previousCgo = $env:CGO_ENABLED

Push-Location -LiteralPath $PSScriptRoot
try {
    $env:CGO_ENABLED = '0'
    & $go build -buildvcs=false -trimpath -ldflags '-s -w -H=windowsgui' -o $extractor .
    if ($LASTEXITCODE -ne 0) { throw "Go build failed with exit code $LASTEXITCODE." }
    [NestedUnpack.BuildResources]::EmbedManifest($extractor, [IO.File]::ReadAllBytes($manifest))

    $destination = Join-Path $output 'NestedUnpack.exe'
    [IO.File]::Copy($extractor, $destination, $true)
    $destination
} finally {
    $env:CGO_ENABLED = $previousCgo
    Pop-Location
    [IO.File]::Delete($extractor)
    [IO.Directory]::Delete($stage, $false)
}
