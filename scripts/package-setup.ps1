param(
    [string]$PackagePath,
    [Parameter(Mandatory)][string]$PublicCertificatePath,
    [string]$OutputDirectory
)
$ErrorActionPreference = 'Stop'
$repoDirectory = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$maximumPackageBytes = 128 * 1024 * 1024
$maximumCertificateBytes = 64 * 1024

function Resolve-SetupInputFile([string]$Value, [string]$Extension) {
    $entry = Get-Item -LiteralPath $Value -Force
    if ($entry -isnot [IO.FileInfo] -or
        ($entry.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0 -or
        $entry.Extension -ine $Extension) {
        throw "Fichier $Extension requis, sans lien : $Value"
    }
    return [IO.Path]::GetFullPath($entry.FullName)
}

function Get-SetupStreamSha256([IO.Stream]$Stream) {
    $sha256 = [Security.Cryptography.SHA256]::Create()
    try {
        $Stream.Position = 0
        return [BitConverter]::ToString($sha256.ComputeHash($Stream)).Replace('-', '').ToLowerInvariant()
    } finally {
        $sha256.Dispose()
        $Stream.Position = 0
    }
}

$cargoVersionMatch = [regex]::Match(
    (Get-Content -LiteralPath (Join-Path $repoDirectory 'Cargo.toml') -Raw),
    '(?m)^version\s*=\s*"([0-9]+\.[0-9]+\.[0-9]+)"\s*$')
if (-not $cargoVersionMatch.Success) { throw 'Version Cargo major.minor.patch introuvable.' }
$appVersion = $cargoVersionMatch.Groups[1].Value
$packageVersion = $appVersion + '.0'
$parsedVersion = [Version]::Parse($packageVersion)
if (@($parsedVersion.Major, $parsedVersion.Minor, $parsedVersion.Build, $parsedVersion.Revision |
        Where-Object { $_ -gt 65535 }).Count -gt 0) {
    throw 'La version MSIX doit contenir quatre nombres de 0 à 65535.'
}
[xml]$projectManifest = Get-Content -LiteralPath (Join-Path $repoDirectory 'packaging\AppxManifest.xml') -Raw
$identity = $projectManifest.Package.Identity
if ([string]$identity.Version -ne $packageVersion -or
    [string]$identity.Name -ne 'Aleqsd.MayhemLens' -or
    [string]$identity.Publisher -ne 'CN=Alexandre DO-O ALMEIDA' -or
    [string]$identity.ProcessorArchitecture -ne 'x64') {
    throw 'Les versions Cargo/MSIX et l''identité x64 attendue doivent correspondre.'
}
if (-not $PackagePath) {
    $PackagePath = Join-Path $repoDirectory ('dist\MayhemLens_' + $packageVersion + '_x64.msix')
}
if (-not $OutputDirectory) { $OutputDirectory = Join-Path $repoDirectory 'dist' }
$PackagePath = Resolve-SetupInputFile $PackagePath '.msix'
$PublicCertificatePath = Resolve-SetupInputFile $PublicCertificatePath '.cer'
$OutputDirectory = $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($OutputDirectory)
if (Test-Path -LiteralPath $OutputDirectory) {
    if ((Get-Item -LiteralPath $OutputDirectory -Force) -isnot [IO.DirectoryInfo]) {
        throw 'Le dossier de sortie est un fichier.'
    }
}

$packageStream = $null
$certificateStream = $null
$certificate = $null
$environmentNames = @('MAYHEM_SETUP_MSIX_PATH', 'MAYHEM_SETUP_CERT_PATH',
    'MAYHEM_SETUP_MSIX_SHA256', 'MAYHEM_SETUP_CERT_SHA256')
$previousEnvironment = @{}
foreach ($name in $environmentNames) {
    $previousEnvironment[$name] = [Environment]::GetEnvironmentVariable($name, 'Process')
}
try {
    # Hold read-only handles until Cargo has embedded the exact hashed payloads.
    # FileShare.Read allows the compiler to read them, while refusing writes/deletes.
    $packageStream = [IO.File]::Open($PackagePath, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::Read)
    $certificateStream = [IO.File]::Open($PublicCertificatePath, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::Read)
    if ($packageStream.Length -lt 1 -or $packageStream.Length -gt $maximumPackageBytes) {
        throw 'Le MSIX doit mesurer de 1 octet à 128 Mio.'
    }
    if ($certificateStream.Length -lt 1 -or $certificateStream.Length -gt $maximumCertificateBytes) {
        throw 'Le certificat public doit mesurer de 1 octet à 64 Kio.'
    }

    Add-Type -AssemblyName System.IO.Compression
    $archive = [IO.Compression.ZipArchive]::new($packageStream, [IO.Compression.ZipArchiveMode]::Read, $true)
    try {
        $entries = @($archive.Entries | Where-Object FullName -ceq 'AppxManifest.xml')
        if ($entries.Count -ne 1 -or $entries[0].Length -gt 64 * 1024) {
            throw 'Manifeste MSIX absent, dupliqué ou trop volumineux.'
        }
        $xmlSettings = [Xml.XmlReaderSettings]::new()
        $xmlSettings.DtdProcessing = [Xml.DtdProcessing]::Prohibit
        $xmlSettings.XmlResolver = $null
        $xmlSettings.MaxCharactersInDocument = 64 * 1024
        $entryStream = $entries[0].Open()
        $reader = $null
        try {
            $reader = [Xml.XmlReader]::Create($entryStream, $xmlSettings)
            $embeddedManifest = [Xml.XmlDocument]::new()
            $embeddedManifest.XmlResolver = $null
            $embeddedManifest.Load($reader)
            $embeddedIdentity = $embeddedManifest.Package.Identity
            foreach ($field in @('Name', 'Publisher', 'Version', 'ProcessorArchitecture')) {
                if ([string]$embeddedIdentity.$field -cne [string]$identity.$field) {
                    throw "L'identité du MSIX intégré ne correspond pas au projet : $field."
                }
            }
        } finally {
            if ($reader) { $reader.Dispose() }
            $entryStream.Dispose()
        }
    } finally { $archive.Dispose() }

    $certificateReader = [IO.BinaryReader]::new($certificateStream, [Text.Encoding]::UTF8, $true)
    try { $certificateBytes = $certificateReader.ReadBytes([int]$certificateStream.Length) }
    finally { $certificateReader.Dispose() }
    $contentType = [Security.Cryptography.X509Certificates.X509Certificate2]::GetCertContentType($certificateBytes)
    if ($contentType -ne [Security.Cryptography.X509Certificates.X509ContentType]::Cert) {
        throw 'Seul un certificat public DER .cer est accepté ; aucune clé/PFX.'
    }
    $certificate = [Security.Cryptography.X509Certificates.X509Certificate2]::new($certificateBytes)
    if ($certificate.HasPrivateKey -or
        $certificate.Subject -cne [string]$identity.Publisher -or
        [Convert]::ToBase64String($certificate.RawData) -cne [Convert]::ToBase64String($certificateBytes)) {
        throw 'Le certificat doit être public seul et correspondre au Publisher du MSIX.'
    }
    if ($certificate.NotAfter.ToUniversalTime() -le [DateTime]::UtcNow -or
        $certificate.NotBefore.ToUniversalTime() -gt [DateTime]::UtcNow) {
        throw 'Le certificat public est expiré ou pas encore valable.'
    }
    $packageHash = Get-SetupStreamSha256 $packageStream
    $certificateHash = Get-SetupStreamSha256 $certificateStream
    [Environment]::SetEnvironmentVariable('MAYHEM_SETUP_MSIX_PATH', $PackagePath, 'Process')
    [Environment]::SetEnvironmentVariable('MAYHEM_SETUP_CERT_PATH', $PublicCertificatePath, 'Process')
    [Environment]::SetEnvironmentVariable('MAYHEM_SETUP_MSIX_SHA256', $packageHash, 'Process')
    [Environment]::SetEnvironmentVariable('MAYHEM_SETUP_CERT_SHA256', $certificateHash, 'Process')
    Push-Location $repoDirectory
    try {
        $metadata = cargo metadata --no-deps --format-version 1 --locked | ConvertFrom-Json
        if ($LASTEXITCODE -ne 0) { throw 'Lecture du dossier de compilation Cargo échouée.' }
        cargo build --release --locked --bin mayhem-lens-setup
        if ($LASTEXITCODE -ne 0) { throw 'Compilation de l''installateur échouée.' }
    } finally { Pop-Location }
    $releaseExe = Join-Path $metadata.target_directory 'release\mayhem-lens-setup.exe'
    if (-not (Test-Path -LiteralPath $releaseExe -PathType Leaf)) {
        throw 'Installateur compilé introuvable.'
    }
    New-Item -ItemType Directory -Path $OutputDirectory -Force | Out-Null
    $setupName = 'MayhemLens-Setup-' + $appVersion + '.exe'
    $setupPath = Join-Path $OutputDirectory $setupName
    Copy-Item -LiteralPath $releaseExe -Destination $setupPath -Force
    # This is a packaging receipt, not a signature, trust or installation proof.
    [pscustomobject]@{
        version = $appVersion
        setupFile = $setupName
        setupBytes = (Get-Item -LiteralPath $setupPath).Length
        setupSha256 = (Get-FileHash -LiteralPath $setupPath -Algorithm SHA256).Hash.ToLowerInvariant()
        packageBytes = $packageStream.Length
        packageSha256 = $packageHash
        certificateBytes = $certificateStream.Length
        certificateSha256 = $certificateHash
        certificateThumbprint = $certificate.Thumbprint
        installerSignedByThisScript = $false
        installationPerformed = $false
        certificateTrustChanged = $false
    } | ConvertTo-Json
} finally {
    foreach ($name in $environmentNames) {
        [Environment]::SetEnvironmentVariable($name, $previousEnvironment[$name], 'Process')
    }
    if ($certificate) { $certificate.Dispose() }
    if ($certificateStream) { $certificateStream.Dispose() }
    if ($packageStream) { $packageStream.Dispose() }
}
