param(
    [Parameter(Mandatory = $true)][string]$OutputDirectory,
    [string]$ArchivePath
)
$ErrorActionPreference = 'Stop'
$projectRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$cacheRoot = Join-Path $projectRoot '.local/export-tools'
$outputRoot = [IO.Path]::GetFullPath($OutputDirectory)
$bundlePath = Join-Path $outputRoot 'export-tools'
$packageName = 'ffmpeg-n8.1.3-18-g46d899d7a0-win64-lgpl-shared-8.1'
$packageHash = '38dd3fb973c2e10723489e2173135d60f569953e70a04b885446175f9e9380d6'
$release = 'https://github.com/BtbN/FFmpeg-Builds/releases/download/autobuild-2026-10-14-13-03'
$ffmpegCommit = '46d899d7a02bc63237472cc4897258d5ddf274ac'
$buildCommit = '63ac3265b0789ea16c7b1f8351cd2ad195a28722'
if (Test-Path -LiteralPath $bundlePath) { throw "Export tools already exist at $bundlePath; choose a fresh output directory" }
New-Item -ItemType Directory -Force -Path $cacheRoot | Out-Null
if (-not $ArchivePath) {
    $ArchivePath = Join-Path $cacheRoot "$packageName.zip"
    if (-not (Test-Path -LiteralPath $ArchivePath)) { Invoke-WebRequest "$release/$packageName.zip" -OutFile $ArchivePath }
}
if ((Get-FileHash -LiteralPath $ArchivePath -Algorithm SHA256).Hash -ne $packageHash) { throw 'FFmpeg archive checksum does not match the pinned package' }
$stagePath = Join-Path $cacheRoot ('stage-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $stagePath | Out-Null
try {
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $archive = [IO.Compression.ZipFile]::OpenRead([IO.Path]::GetFullPath($ArchivePath))
    try {
        foreach ($entry in $archive.Entries) {
            $resolved = [IO.Path]::GetFullPath((Join-Path $stagePath $entry.FullName))
            if (-not $resolved.StartsWith($stagePath + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) { throw 'Archive entry escapes the staging directory' }
        }
    } finally { $archive.Dispose() }
    Expand-Archive -LiteralPath $ArchivePath -DestinationPath $stagePath
    $unpacked = Join-Path $stagePath $packageName
    $version = (& (Join-Path $unpacked 'bin/ffmpeg.exe') -version 2>&1 | Out-String)
    if ($LASTEXITCODE -ne 0) { throw 'The pinned encoder could not start' }
    if ($version -notmatch [regex]::Escape('n8.1.3-18-g46d899d7a0') -or $version -match '--enable-gpl\b|--enable-nonfree\b') { throw 'Unexpected encoder version or license configuration' }
    $license = (& (Join-Path $unpacked 'bin/ffmpeg.exe') -hide_banner -L 2>&1 | Out-String)
    if ($LASTEXITCODE -ne 0 -or $license -notmatch 'GNU Lesser General Public License') { throw 'The encoder did not report its LGPL license' }
    $prepared = Join-Path $stagePath 'export-tools'
    New-Item -ItemType Directory -Path $prepared | Out-Null
    Copy-Item -LiteralPath (Join-Path $unpacked 'bin/ffmpeg.exe') -Destination $prepared
    Get-ChildItem -LiteralPath (Join-Path $unpacked 'bin') -Filter '*.dll' -File | ForEach-Object { Copy-Item -LiteralPath $_.FullName -Destination $prepared }
    Copy-Item -LiteralPath (Join-Path $unpacked 'LICENSE.txt') -Destination (Join-Path $prepared 'COPYING.LGPLv3')
    $version | Set-Content -LiteralPath (Join-Path $prepared 'BUILD.txt') -Encoding utf8
    $sourceRoot = Join-Path $prepared 'sources'
    New-Item -ItemType Directory -Path $sourceRoot | Out-Null
    Invoke-WebRequest "https://github.com/FFmpeg/FFmpeg/archive/$ffmpegCommit.tar.gz" -OutFile (Join-Path $sourceRoot 'ffmpeg.tar.gz')
    Invoke-WebRequest "https://github.com/BtbN/FFmpeg-Builds/archive/$buildCommit.tar.gz" -OutFile (Join-Path $sourceRoot 'build-recipes.tar.gz')
    Invoke-WebRequest "https://raw.githubusercontent.com/FFmpeg/FFmpeg/$ffmpegCommit/COPYING.GPLv3" -OutFile (Join-Path $prepared 'COPYING.GPLv3')
    @"
Sift audio export uses FFmpeg under LGPL version 3 or later.
FFmpeg copyright: the FFmpeg developers. https://ffmpeg.org/
Build provider: BtbN/FFmpeg-Builds, snapshot autobuild-2026-10-14-13-03.
Binary archive SHA-256: $packageHash
FFmpeg source commit: $ffmpegCommit
Build recipe commit: $buildCommit
FFmpeg source and recipe archives are included in sources/.
The complete build configuration is recorded in BUILD.txt.
Before redistributing a release, finish corresponding-source and notice coverage
for every enabled third-party dependency. See docs/EXPORT-TOOLS.md in Sift.
"@ | Set-Content -LiteralPath (Join-Path $prepared 'NOTICE.txt') -Encoding utf8
    $manifest = Get-ChildItem -LiteralPath $prepared -Recurse -File | ForEach-Object {
        [pscustomobject]@{ path = [IO.Path]::GetRelativePath($prepared, $_.FullName); sha256 = (Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant() }
    }
    $manifest | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $prepared 'manifest.json') -Encoding utf8
    New-Item -ItemType Directory -Force -Path $outputRoot | Out-Null
    if (-not ([IO.Path]::GetFullPath($prepared)).StartsWith($stagePath + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase) -or (Join-Path $outputRoot 'export-tools') -ne $bundlePath) { throw 'Unexpected publication path' }
    Move-Item -LiteralPath $prepared -Destination $bundlePath
    Write-Output "Prepared local LGPL export tools: $bundlePath"
} finally {
    $resolvedStage = [IO.Path]::GetFullPath($stagePath)
    if (-not $resolvedStage.StartsWith($cacheRoot + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase) -or [IO.Path]::GetFileName($resolvedStage) -notmatch '^stage-[0-9a-f]{32}$') { throw 'Refusing cleanup outside the owned staging directory' }
    Remove-Item -LiteralPath $resolvedStage -Recurse -Force
}
