param(
    [Parameter(Mandatory)][string]$PackagePath,
    [Parameter(Mandatory)][string]$SigningDirectory,
    [Parameter(Mandatory)][string]$PublicCertificatePath
)
$ErrorActionPreference = 'Stop'
$PackagePath = [IO.Path]::GetFullPath($PackagePath)
$SigningDirectory = [IO.Path]::GetFullPath($SigningDirectory)
$PublicCertificatePath = [IO.Path]::GetFullPath($PublicCertificatePath)
if (-not (Test-Path -LiteralPath $PackagePath)) { throw 'MSIX absent.' }
New-Item -ItemType Directory -Path $SigningDirectory -Force | Out-Null
$privatePath = Join-Path $SigningDirectory 'development-key.dpapi'
$scope = [Security.Cryptography.DataProtectionScope]::CurrentUser
if (Test-Path -LiteralPath $privatePath) {
    $privateBytes = [Security.Cryptography.ProtectedData]::Unprotect([IO.File]::ReadAllBytes($privatePath),$null,$scope)
    $certificate = [Security.Cryptography.X509Certificates.X509Certificate2]::new(
        $privateBytes,'',[Security.Cryptography.X509Certificates.X509KeyStorageFlags]::EphemeralKeySet -bor
        [Security.Cryptography.X509Certificates.X509KeyStorageFlags]::Exportable)
    if ($certificate.NotAfter -le [DateTime]::UtcNow.AddDays(1)) { throw 'Certificat de développement expiré ou proche de son expiration.' }
} else {
    $rsa = [Security.Cryptography.RSA]::Create(2048)
    $request = [Security.Cryptography.X509Certificates.CertificateRequest]::new(
        'CN=Alexandre DO-O ALMEIDA',$rsa,[Security.Cryptography.HashAlgorithmName]::SHA256,
        [Security.Cryptography.RSASignaturePadding]::Pkcs1)
    $request.CertificateExtensions.Add([Security.Cryptography.X509Certificates.X509BasicConstraintsExtension]::new($false,$false,0,$true))
    $request.CertificateExtensions.Add([Security.Cryptography.X509Certificates.X509KeyUsageExtension]::new(
        [Security.Cryptography.X509Certificates.X509KeyUsageFlags]::DigitalSignature,$true))
    $oids = [Security.Cryptography.OidCollection]::new()
    $oids.Add([Security.Cryptography.Oid]::new('1.3.6.1.5.5.7.3.3')) | Out-Null
    $request.CertificateExtensions.Add([Security.Cryptography.X509Certificates.X509EnhancedKeyUsageExtension]::new($oids,$true))
    $request.CertificateExtensions.Add([Security.Cryptography.X509Certificates.X509SubjectKeyIdentifierExtension]::new($request.PublicKey,$false))
    $certificate = $request.CreateSelfSigned([DateTimeOffset]::UtcNow.AddMinutes(-5),[DateTimeOffset]::UtcNow.AddYears(1))
    $privateBytes = $certificate.Export([Security.Cryptography.X509Certificates.X509ContentType]::Pfx,'')
    [IO.File]::WriteAllBytes($privatePath,[Security.Cryptography.ProtectedData]::Protect($privateBytes,$null,$scope))
    $rsa.Dispose()
}
# SignTool needs a short-lived PFX. The saved key is encrypted by Windows for the
# current user; no private material or password is printed or placed in a package.
$temporaryPfx = Join-Path $SigningDirectory ([Guid]::NewGuid().ToString()+'.pfx')
try {
    [IO.File]::WriteAllBytes($temporaryPfx,$privateBytes)
    [IO.File]::WriteAllBytes($PublicCertificatePath,$certificate.Export([Security.Cryptography.X509Certificates.X509ContentType]::Cert))
    $sdkDirectory = Join-Path ${env:ProgramFiles(x86)} 'Windows Kits\10\bin'
    $signTool = Get-ChildItem -LiteralPath $sdkDirectory -Filter SignTool.exe -Recurse |
        Where-Object { $_.Directory.Name -eq 'x64' } | Sort-Object FullName -Descending | Select-Object -First 1
    if (-not $signTool) { throw 'SignTool absent du Windows SDK.' }
    & $signTool.FullName sign /fd SHA256 /f $temporaryPfx $PackagePath
    if ($LASTEXITCODE -ne 0) { throw 'La signature MSIX a échoué.' }
    Write-Output ('Certificat public : '+$PublicCertificatePath)
    Write-Output ('Empreinte du certificat : '+$certificate.Thumbprint)
    Write-Output 'Signature de développement ajoutée. Aucun certificat approuvé, aucune installation et aucun overlay lancé.'
} finally {
    if (Test-Path -LiteralPath $temporaryPfx) { Remove-Item -LiteralPath $temporaryPfx }
    if ($privateBytes) { [Array]::Clear($privateBytes,0,$privateBytes.Length) }
    $certificate.Dispose()
}
