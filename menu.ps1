[CmdletBinding()]
param([switch]$Remove)

$ErrorActionPreference = 'Stop'
$registry = [Microsoft.Win32.Registry]::CurrentUser
$verbKey = 'Software\Classes\*\shell\NestedUnpack.Extract'
$clsid = '{9E941531-6A19-4358-A773-30CD664BE034}'
$classKey = "Software\Classes\CLSID\$clsid"

if ($Remove) {
    $registry.DeleteSubKeyTree($verbKey, $false)
    $registry.DeleteSubKeyTree($classKey, $false)
} else {
    $bin = Join-Path $PSScriptRoot 'bin'
    $extractor = Join-Path $bin 'NestedUnpack.exe'
    $server = Join-Path $bin 'NestedUnpackShell.exe'
    foreach ($file in @($extractor, $server)) {
        if (-not (Test-Path -LiteralPath $file -PathType Leaf)) {
            throw "Missing executable: $file"
        }
    }

    $serverKey = $registry.CreateSubKey("$classKey\LocalServer32")
    try {
        $serverKey.SetValue('', ('"{0}" --nested' -f $server))
        $serverKey.SetValue('ServerExecutable', $server)
    } finally { $serverKey.Dispose() }

    $verb = $registry.CreateSubKey($verbKey)
    try {
        $label = -join [char[]](0x89e3,0x5f00,0x5d4c,0x5957,0x538b,0x7f29,0x5305)
        $verb.SetValue('MUIVerb', $label)
        $verb.SetValue('NeverDefault', '')
        $verb.SetValue('MultiSelectModel', 'Player')
        $verb.SetValue('AppliesTo', 'System.FileExtension:=".zip" OR System.FileExtension:=".mp4"')
        $command = $verb.CreateSubKey('command')
        try { $command.SetValue('DelegateExecute', $clsid) }
        finally { $command.Dispose() }
    } finally { $verb.Dispose() }
}

# Remove menu entries registered before the rename.
$registry.DeleteSubKeyTree('Software\Classes\*\shell\OuterRar.ExtractNested', $false)
$registry.DeleteSubKeyTree('Software\Classes\*\shell\OuterRar.Extract', $false)
$registry.DeleteSubKeyTree('Software\Classes\CLSID\{DF5835F3-405C-4950-94CE-121C0EA39879}', $false)

if (-not ('NestedUnpack.AssociationChange' -as [type])) {
    Add-Type -Namespace NestedUnpack -Name AssociationChange -MemberDefinition @'
        [System.Runtime.InteropServices.DllImport("shell32.dll")]
        public static extern void SHChangeNotify(uint eventId, uint flags,
                                                IntPtr item1, IntPtr item2);
'@
}
[NestedUnpack.AssociationChange]::SHChangeNotify(0x08000000, 0, [IntPtr]::Zero, [IntPtr]::Zero)

if ($Remove) { 'NestedUnpack Explorer command removed.' }
else { 'NestedUnpack Explorer command registered for .zip and .mp4.' }
