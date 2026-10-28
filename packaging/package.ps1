<#
.SYNOPSIS
    Builds DropReel and packages it with libmpv, ffmpeg and ffprobe.

.DESCRIPTION
    Stages (default: all):
      deps     Download libmpv-2.dll (shinchiro/mpv-winbuild-cmake) and ffmpeg.exe/ffprobe.exe with
               their DLLs (BtbN/FFmpeg-Builds, GPL shared build) into packaging\deps. Skipped if
               already there.
      build    cargo build --release
      package  Assemble dist\DropReel\, zip it to dist\DropReel-<version>-win64.zip and, if Inno
               Setup 6 is installed, build dist\DropReel-<version>-setup.exe.

    Needs: Rust (rustup), and 7-Zip or a tar.exe that can read .7z (Windows 10+ includes one).

.EXAMPLE
    .\packaging\package.ps1
.EXAMPLE
    .\packaging\package.ps1 -Stage deps
.EXAMPLE
    .\packaging\package.ps1 -MpvDevUrl https://example.org/mpv-dev-x86_64-20250101-git-abcdef0.7z
#>
[CmdletBinding()]
param(
    [ValidateSet('all', 'deps', 'build', 'package')]
    [string]$Stage = 'all',
    # A specific mpv-dev-x86_64-*.7z; by default the newest shinchiro release is used.
    [string]$MpvDevUrl = '',
    # The "shared" build keeps the FFmpeg libraries in DLLs that ffmpeg.exe and ffprobe.exe share,
    # which makes the package about half the size of two static executables.
    [string]$FfmpegUrl = 'https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-win64-gpl-shared.zip',
    [switch]$NoInstaller
)

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'  # Invoke-WebRequest is much faster without the progress bar

$root = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$deps = Join-Path $PSScriptRoot 'deps'
$dist = Join-Path $root 'dist'
$version = (Select-String -Path (Join-Path $root 'Cargo.toml') -Pattern '^version\s*=\s*"([^"]+)"' |
    Select-Object -First 1).Matches[0].Groups[1].Value
# A fourth version number is Cargo build metadata ("0.3.5+1"); files and the installer say "0.3.5.1".
$version = $version -replace '\+', '.'

function Invoke-GitHubApi([string]$Url) {
    $headers = @{ 'User-Agent' = 'dropreel-packaging'; 'Accept' = 'application/vnd.github+json' }
    if ($env:GITHUB_TOKEN) { $headers['Authorization'] = "Bearer $env:GITHUB_TOKEN" }
    Invoke-RestMethod -Uri $Url -Headers $headers
}

function Expand-7z([string]$Archive, [string]$Destination) {
    New-Item -ItemType Directory -Force $Destination | Out-Null
    $sevenZip = (Get-Command 7z -ErrorAction SilentlyContinue).Source
    if (-not $sevenZip -and (Test-Path "$env:ProgramFiles\7-Zip\7z.exe")) { $sevenZip = "$env:ProgramFiles\7-Zip\7z.exe" }
    if ($sevenZip) {
        & $sevenZip x $Archive "-o$Destination" -y | Out-Null
        if ($LASTEXITCODE) { throw "7-Zip could not extract $Archive" }
    } else {
        tar -xf $Archive -C $Destination
        if ($LASTEXITCODE) { throw "Could not extract $Archive. Install 7-Zip (https://7-zip.org) and run again." }
    }
}

function Get-Deps {
    New-Item -ItemType Directory -Force $deps | Out-Null
    $work = Join-Path $deps '_download'
    New-Item -ItemType Directory -Force $work | Out-Null

    if (-not (Test-Path (Join-Path $deps 'libmpv-2.dll'))) {
        $url = $MpvDevUrl
        if (-not $url) {
            $release = Invoke-GitHubApi 'https://api.github.com/repos/shinchiro/mpv-winbuild-cmake/releases/latest'
            # The plain x86_64 build (not "-v3", which needs AVX2).
            $asset = $release.assets | Where-Object { $_.name -match '^mpv-dev-x86_64-\d{8}-git-[0-9a-f]+\.7z$' } | Select-Object -First 1
            if (-not $asset) { throw "No mpv-dev-x86_64 asset in shinchiro/mpv-winbuild-cmake $($release.tag_name). Pass -MpvDevUrl." }
            $url = $asset.browser_download_url
        }
        Write-Host "Downloading libmpv from $url"
        $archive = Join-Path $work 'mpv-dev.7z'
        Invoke-WebRequest -Uri $url -OutFile $archive
        $out = Join-Path $work 'mpv'
        Expand-7z $archive $out
        $dll = Get-ChildItem $out -Recurse -Filter 'libmpv-2.dll' | Select-Object -First 1
        if (-not $dll) { throw 'libmpv-2.dll was not found in the mpv-dev archive' }
        Copy-Item $dll.FullName $deps
        Set-Content -Path (Join-Path $deps 'libmpv.source.txt') -Value $url
    }

    $ffmpegMarker = Join-Path $deps 'ffmpeg.source.txt'
    $haveFfmpeg = (Test-Path (Join-Path $deps 'ffprobe.exe')) -and (Test-Path $ffmpegMarker) -and
        ((Get-Content $ffmpegMarker -Raw).Trim() -eq $FfmpegUrl)
    if (-not $haveFfmpeg) {
        # Remove any previous ffmpeg build (static or shared) before unpacking the new one.
        Get-ChildItem $deps -File | Where-Object { $_.Name -match '^(ffmpeg|ffprobe)\.exe$|^(av|sw|postproc).*\.dll$' } | Remove-Item -Force
        Write-Host "Downloading ffmpeg from $FfmpegUrl"
        $zip = Join-Path $work 'ffmpeg.zip'
        Invoke-WebRequest -Uri $FfmpegUrl -OutFile $zip
        $out = Join-Path $work 'ffmpeg'
        Expand-Archive -Path $zip -DestinationPath $out -Force
        $ffmpegExe = Get-ChildItem $out -Recurse -Filter 'ffmpeg.exe' | Select-Object -First 1
        if (-not $ffmpegExe) { throw 'ffmpeg.exe was not found in the ffmpeg archive' }
        $bin = $ffmpegExe.Directory.FullName
        foreach ($exe in 'ffmpeg.exe', 'ffprobe.exe') {
            if (-not (Test-Path (Join-Path $bin $exe))) { throw "$exe was not found in the ffmpeg archive" }
            Copy-Item (Join-Path $bin $exe) $deps
        }
        # Shared builds: the FFmpeg libraries both executables need.
        Get-ChildItem $bin -Filter '*.dll' | Copy-Item -Destination $deps
        $license = Get-ChildItem $out -Recurse -Filter 'LICENSE*' | Select-Object -First 1
        if ($license) { Copy-Item $license.FullName (Join-Path $deps 'ffmpeg-LICENSE.txt') }
        Set-Content -Path (Join-Path $deps 'ffmpeg.source.txt') -Value $FfmpegUrl
    }

    Remove-Item $work -Recurse -Force -ErrorAction SilentlyContinue
    Write-Host "Third-party binaries are in $deps"
}

function Invoke-Build {
    Push-Location $root
    try {
        cargo build --release --locked
        if ($LASTEXITCODE) { throw 'cargo build failed' }
    } finally { Pop-Location }
}

function Write-CrateList([string]$Path) {
    $header = "Rust crates compiled into DropReel.exe (name, version, license).`r`n" +
              "Their full license texts are in each crate's source on https://crates.io.`r`n"
    try {
        $json = cargo metadata --format-version 1 --locked --filter-platform x86_64-pc-windows-msvc --manifest-path (Join-Path $root 'Cargo.toml')
        $meta = $json | ConvertFrom-Json
        $lines = $meta.packages | Where-Object { $_.name -ne 'dropreel' } | Sort-Object name, version | ForEach-Object {
            $license = if ($_.license) { $_.license } else { 'see crate' }
            '{0} {1} - {2}' -f $_.name, $_.version, $license
        }
        Set-Content -Path $Path -Value ($header + "`r`n" + ($lines -join "`r`n")) -Encoding utf8
    } catch {
        Set-Content -Path $Path -Value ($header + "`r`n(Run with PowerShell 7 to generate the list; see Cargo.lock.)") -Encoding utf8
    }
}

function New-Package {
    $exe = Join-Path $root 'target\release\DropReel.exe'
    if (-not (Test-Path $exe)) { throw "$exe is missing. Run the build stage first." }
    foreach ($file in 'libmpv-2.dll', 'ffmpeg.exe', 'ffprobe.exe') {
        if (-not (Test-Path (Join-Path $deps $file))) { throw "$file is missing. Run the deps stage first." }
    }

    $stage = Join-Path $dist 'DropReel'
    if (Test-Path $stage) { Remove-Item $stage -Recurse -Force }
    $licenses = Join-Path $stage 'licenses'
    New-Item -ItemType Directory -Force $licenses | Out-Null

    Copy-Item $exe $stage
    foreach ($file in 'ffmpeg.exe', 'ffprobe.exe') { Copy-Item (Join-Path $deps $file) $stage }
    # libmpv-2.dll plus, for a shared ffmpeg build, its avcodec/avformat/... DLLs.
    Get-ChildItem $deps -Filter '*.dll' | Copy-Item -Destination $stage

    Copy-Item (Join-Path $root 'LICENSE') (Join-Path $licenses 'DropReel-LICENSE.txt')
    Copy-Item (Join-Path $root 'THIRD-PARTY-NOTICES.md') $licenses
    Copy-Item (Join-Path $root 'assets\fonts\LICENSE-Inter.txt') $licenses
    Copy-Item (Join-Path $root 'assets\fonts\LICENSE-Phosphor.txt') $licenses
    $ffmpegLicense = Join-Path $deps 'ffmpeg-LICENSE.txt'
    if (Test-Path $ffmpegLicense) { Copy-Item $ffmpegLicense $licenses }
    foreach ($source in 'libmpv.source.txt', 'ffmpeg.source.txt') {
        $p = Join-Path $deps $source
        if (Test-Path $p) { Copy-Item $p $licenses }
    }
    Write-CrateList (Join-Path $licenses 'rust-crates.txt')

    $readme = @"
DropReel $version
=================

Run DropReel.exe. Drag movie files or folders onto the window to add them.

Keep the other files next to DropReel.exe:
  - libmpv-2.dll  plays movies inside the app
  - ffmpeg.exe    makes thumbnails and re-encodes large files to HEVC
  - ffprobe.exe   reads durations, codecs and tracks
  - av*.dll, sw*.dll  FFmpeg libraries used by ffmpeg.exe and ffprobe.exe

Your library and settings live in %APPDATA%\DropReel.
Licenses for DropReel and the bundled components are in the licenses folder.
More: https://github.com/alister-o7/dropreel
"@
    Set-Content -Path (Join-Path $stage 'README.txt') -Value $readme -Encoding utf8

    $zip = Join-Path $dist "DropReel-$version-win64.zip"
    if (Test-Path $zip) { Remove-Item $zip -Force }
    Compress-Archive -Path (Join-Path $stage '*') -DestinationPath $zip
    Write-Host "Created $zip"

    if ($NoInstaller) { return }
    $iscc = (Get-Command iscc -ErrorAction SilentlyContinue).Source
    foreach ($candidate in "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe", "$env:ProgramFiles\Inno Setup 6\ISCC.exe", "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe") {
        if (-not $iscc -and (Test-Path $candidate)) { $iscc = $candidate }
    }
    if (-not $iscc) {
        Write-Host 'Inno Setup 6 not found; skipping the installer (https://jrsoftware.org/isinfo.php).'
        return
    }
    & $iscc "/DAppVersion=$version" "/DSourceDir=$stage" "/DOutputDir=$dist" (Join-Path $PSScriptRoot 'dropreel.iss')
    if ($LASTEXITCODE) { throw 'Inno Setup failed' }
    Write-Host "Created $(Join-Path $dist "DropReel-$version-setup.exe")"
}

switch ($Stage) {
    'deps' { Get-Deps }
    'build' { Invoke-Build }
    'package' { New-Package }
    'all' { Get-Deps; Invoke-Build; New-Package }
}
