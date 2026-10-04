param(
    [string]$OutputDirectory,
    [switch]$SkipBuild
)
$ErrorActionPreference = 'Stop'
$repoDirectory = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
if (-not $OutputDirectory) { $OutputDirectory = Join-Path $repoDirectory 'dist' }
$OutputDirectory = [IO.Path]::GetFullPath($OutputDirectory)
[xml]$manifest = Get-Content -LiteralPath (Join-Path $repoDirectory 'packaging\AppxManifest.xml') -Raw
$packageVersion = [string]$manifest.Package.Identity.Version
$rustVersionMatch = [regex]::Match((Get-Content -LiteralPath (Join-Path $repoDirectory 'Cargo.toml') -Raw), '(?m)^version = "([0-9]+\.[0-9]+\.[0-9]+)"')
if (-not $rustVersionMatch.Success -or $packageVersion -ne ($rustVersionMatch.Groups[1].Value + '.0')) {
    throw 'Les versions Cargo et MSIX doivent correspondre (major.minor.patch.0).'
}
$packageFileName = 'MayhemLens_' + $packageVersion + '_x64.msix'
$stagingName = '.package-staging-' + [Guid]::NewGuid().ToString('N')
$packageDirectory = [IO.Path]::GetFullPath((Join-Path $OutputDirectory $stagingName))
New-Item -ItemType Directory -Path $OutputDirectory -Force | Out-Null
# Each run owns a fresh directory: stale files and directory-copy nesting cannot
# enter the MSIX, and any pre-existing "package" directory is left untouched.
New-Item -ItemType Directory -Path $packageDirectory | Out-Null
try {
    if (-not $SkipBuild) {
        Push-Location $repoDirectory
        try {
            $env:CARGO_BUILD_JOBS = '2'
            cargo build --release --locked
            if ($LASTEXITCODE -ne 0) { throw 'La compilation release a échoué.' }
        } finally { Pop-Location }
    }
    $releaseExe = Join-Path $repoDirectory 'target\release\mayhem-lens.exe'
    $binaryVersion = & $releaseExe --version | Out-String
    if ($LASTEXITCODE -ne 0 -or $binaryVersion.Trim() -ne ('Mayhem Lens ' + $rustVersionMatch.Groups[1].Value)) {
        throw 'La version de l''exécutable release ne correspond pas au manifeste. Recompiler sans SkipBuild.'
    }
    Copy-Item -LiteralPath $releaseExe -Destination $packageDirectory
    Copy-Item -LiteralPath (Join-Path $repoDirectory 'packaging\AppxManifest.xml') -Destination $packageDirectory
    $assetsDirectory = Join-Path $packageDirectory 'Assets'
    New-Item -ItemType Directory -Path $assetsDirectory -Force | Out-Null
    # Locally drawn geometric application marks, without Riot assets or game datasets.
    Add-Type -AssemblyName System.Drawing
    foreach ($asset in @(@('StoreLogo.png',50), @('Square44x44Logo.png',44), @('Square150x150Logo.png',150))) {
        $size = [int]$asset[1]
        $bitmap = [Drawing.Bitmap]::new($size,$size)
        $graphics = [Drawing.Graphics]::FromImage($bitmap)
        $pen = [Drawing.Pen]::new([Drawing.Color]::FromArgb(244,193,74),[single]($size*0.09))
        try {
            $graphics.SmoothingMode = [Drawing.Drawing2D.SmoothingMode]::AntiAlias
            $graphics.Clear([Drawing.Color]::FromArgb(18,24,37))
            $points = [Drawing.PointF[]]@(
                [Drawing.PointF]::new($size*0.24,$size*0.74),
                [Drawing.PointF]::new($size*0.24,$size*0.28),
                [Drawing.PointF]::new($size*0.50,$size*0.57),
                [Drawing.PointF]::new($size*0.76,$size*0.28),
                [Drawing.PointF]::new($size*0.76,$size*0.74))
            $graphics.DrawLines($pen,$points)
            $bitmap.Save((Join-Path $assetsDirectory $asset[0]),[Drawing.Imaging.ImageFormat]::Png)
        } finally { $pen.Dispose(); $graphics.Dispose(); $bitmap.Dispose() }
    }
    # Preserve dependency license files next to the binary. No provider dataset is packaged.
    $noticeDirectory = Join-Path $packageDirectory 'ThirdPartyLicenses'
    New-Item -ItemType Directory -Path $noticeDirectory -Force | Out-Null
    $rustSysroot = rustc --print sysroot
    $rustLicenseDirectory = Join-Path $rustSysroot 'share\doc\rust\licenses'
    if (Test-Path -LiteralPath $rustLicenseDirectory) {
        Copy-Item -LiteralPath $rustLicenseDirectory -Destination (Join-Path $noticeDirectory 'Rust') -Recurse -Force
    }
    $cargoRegistry = Join-Path $env:USERPROFILE '.cargo\registry\src'
    $lockText = Get-Content -LiteralPath (Join-Path $repoDirectory 'Cargo.lock') -Raw
    foreach ($packageMatch in [regex]::Matches($lockText, '(?ms)\[\[package\]\]\r?\nname = "([^"]+)"\r?\nversion = "([^"]+)".*?(?=\[\[package\]\]|\z)')) {
        $packageName = $packageMatch.Groups[1].Value + '-' + $packageMatch.Groups[2].Value
        foreach ($registry in Get-ChildItem -LiteralPath $cargoRegistry -Directory) {
            $sourceDirectory = Join-Path $registry.FullName $packageName
            if (-not (Test-Path -LiteralPath $sourceDirectory)) { continue }
            $licenseFiles = Get-ChildItem -LiteralPath $sourceDirectory -File |
                Where-Object { $_.Name -match '^(LICENSE|LICENCE|COPYING|NOTICE)' }
            if ($licenseFiles.Count -gt 0) {
                $destination = Join-Path $noticeDirectory $packageName
                New-Item -ItemType Directory -Path $destination -Force | Out-Null
                foreach ($license in $licenseFiles) { Copy-Item -LiteralPath $license.FullName -Destination $destination }
            }
        }
    }
    $sdkDirectory = Join-Path ${env:ProgramFiles(x86)} 'Windows Kits\10\bin'
    $makeAppx = Get-ChildItem -LiteralPath $sdkDirectory -Filter MakeAppx.exe -Recurse |
        Where-Object { $_.Directory.Name -eq 'x64' } | Sort-Object FullName -Descending | Select-Object -First 1
    if (-not $makeAppx) { throw 'MakeAppx absent : installer le Windows SDK.' }
    $packagePath = Join-Path $OutputDirectory $packageFileName
    $packLog = Join-Path $OutputDirectory 'packaging.log'
    & $makeAppx.FullName pack /d $packageDirectory /p $packagePath /o *> $packLog
    if ($LASTEXITCODE -ne 0) {
        Get-Content -LiteralPath $packLog -Tail 20
        throw 'Validation ou création du MSIX échouée.'
    }
    Get-FileHash -LiteralPath $packagePath -Algorithm SHA256 | Format-List
    Write-Output 'Package non signé créé. Aucune installation, aucun certificat approuvé, aucun overlay lancé.'
} finally {
    $cleanupDirectory = [IO.Path]::GetFullPath($packageDirectory)
    $cleanupParent = [IO.Path]::GetFullPath((Split-Path -Parent $cleanupDirectory))
    if ($cleanupParent.TrimEnd('\', '/') -ne $OutputDirectory.TrimEnd('\', '/') -or
        (Split-Path -Leaf $cleanupDirectory) -ne $stagingName) {
        throw 'Le chemin du staging ne correspond plus au dossier temporaire créé.'
    }
    if (Test-Path -LiteralPath $cleanupDirectory) {
        $stagingInfo = Get-Item -LiteralPath $cleanupDirectory -Force
        if (-not $stagingInfo.PSIsContainer -or
            ($stagingInfo.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
            throw 'Le staging est devenu un fichier ou un lien : nettoyage refusé.'
        }
        Remove-Item -LiteralPath $cleanupDirectory -Recurse -Force
    }
}
