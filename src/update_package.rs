//! Read-only MSIX identity check before handing a local package to Windows.
//!
//! The caller must initialize COM on this same thread and retain that apartment
//! until this function returns. All streams/readers are released before return;
//! this helper neither initializes COM nor deploys a package. Identity validation
//! does not validate a signature: Windows deployment must use AllowUnsigned=false.

use anyhow::Result;
use std::path::Path;

#[cfg(any(windows, test))]
#[derive(Clone)]
struct PackageIdentity {
    name: String,
    publisher: String,
    version: u64,
    is_x64: bool,
}

#[cfg(any(windows, test))]
fn validate_identity(
    actual: &PackageIdentity,
    expected_name: &str,
    expected_publisher: &str,
    expected_version: [u16; 4],
) -> Result<()> {
    // Downloaded manifest fields are untrusted. Errors deliberately omit them.
    anyhow::ensure!(
        !expected_name.is_empty() && actual.name == expected_name,
        "Nom du package MSIX inattendu"
    );
    anyhow::ensure!(
        !expected_publisher.is_empty() && actual.publisher == expected_publisher,
        "Éditeur du package MSIX inattendu"
    );
    let packed_version = expected_version
        .iter()
        .fold(0_u64, |value, part| (value << 16) | u64::from(*part));
    anyhow::ensure!(
        actual.version == packed_version,
        "Version du package MSIX inattendue"
    );
    anyhow::ensure!(actual.is_x64, "Le package MSIX doit cibler x64");
    Ok(())
}

/// Read a local MSIX through Microsoft's package reader and match its identity.
/// Requires an initialized COM apartment on the calling thread; no installation.
#[cfg(windows)]
pub(crate) fn validate(
    path: &Path,
    expected_name: &str,
    expected_publisher: &str,
    expected_version: [u16; 4],
) -> Result<()> {
    use anyhow::{Context, ensure};
    use std::os::windows::ffi::OsStrExt;
    use windows::{
        Win32::{
            Storage::Packaging::Appx::{APPX_PACKAGE_ARCHITECTURE_X64, AppxFactory, IAppxFactory},
            System::Com::{
                CLSCTX_INPROC_SERVER, CoCreateInstance, STGM_READ, STGM_SHARE_DENY_WRITE,
            },
            UI::Shell::SHCreateStreamOnFileEx,
        },
        core::PCWSTR,
    };

    let mut filename: Vec<u16> = path.as_os_str().encode_wide().collect();
    ensure!(
        !filename.is_empty() && !filename.contains(&0),
        "Chemin du package MSIX invalide"
    );
    filename.push(0);
    // SAFETY: the NUL-terminated UTF-16 buffer remains alive throughout the call.
    // READ + DENY_WRITE opens the existing file only; fCreate=false prevents any
    // creation. The caller initialized COM and owns all returned interfaces here.
    let stream = unsafe {
        SHCreateStreamOnFileEx(
            PCWSTR(filename.as_ptr()),
            (STGM_READ | STGM_SHARE_DENY_WRITE).0,
            0,
            false,
            None,
        )
    }
    .context("Lecture du fichier MSIX")?;
    // SAFETY: CoCreateInstance returns an owned interface from the system's Appx
    // factory; stream, reader and manifest are kept alive until identity is read.
    let factory: IAppxFactory =
        unsafe { CoCreateInstance(&AppxFactory, None, CLSCTX_INPROC_SERVER) }
            .context("Création du lecteur MSIX")?;
    let reader =
        unsafe { factory.CreatePackageReader(&stream) }.context("Package MSIX invalide")?;
    let manifest = unsafe { reader.GetManifest() }.context("Manifeste MSIX indisponible")?;
    let identity = unsafe { manifest.GetPackageId() }.context("Identité MSIX indisponible")?;
    let name =
        TaskString(unsafe { identity.GetName() }.context("Nom MSIX indisponible")?).read()?;
    let publisher =
        TaskString(unsafe { identity.GetPublisher() }.context("Éditeur MSIX indisponible")?)
            .read()?;
    let version = unsafe { identity.GetVersion() }.context("Version MSIX indisponible")?;
    let architecture =
        unsafe { identity.GetArchitecture() }.context("Architecture MSIX indisponible")?;
    validate_identity(
        &PackageIdentity {
            name,
            publisher,
            version,
            is_x64: architecture == APPX_PACKAGE_ARCHITECTURE_X64,
        },
        expected_name,
        expected_publisher,
        expected_version,
    )
}

/// Owns the task-allocated strings returned by IAppxManifestPackageId getters.
#[cfg(windows)]
struct TaskString(windows::core::PWSTR);

#[cfg(windows)]
impl TaskString {
    fn read(&self) -> Result<String> {
        anyhow::ensure!(!self.0.is_null(), "Champ d'identité MSIX absent");
        // SAFETY: the manifest getter supplied an owned, NUL-terminated UTF-16
        // string. Drop frees it even if UTF-16 conversion returns an error.
        unsafe { self.0.to_string() }.map_err(|_| anyhow::anyhow!("Champ d'identité MSIX invalide"))
    }
}

#[cfg(windows)]
impl Drop for TaskString {
    fn drop(&mut self) {
        // SAFETY: only strings allocated by an Appx manifest getter enter this
        // owner; Microsoft requires CoTaskMemFree, which also accepts null.
        unsafe { windows::Win32::System::Com::CoTaskMemFree(Some(self.0.0.cast())) };
    }
}

#[cfg(not(windows))]
// The updater itself is Windows-only; retain an explicit unsupported stub for
// portable builds without requiring those builds to call it.
#[allow(dead_code)]
pub(crate) fn validate(_: &Path, _: &str, _: &str, _: [u16; 4]) -> Result<()> {
    anyhow::bail!("Le contrôle natif du package MSIX nécessite Windows")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity() -> PackageIdentity {
        PackageIdentity {
            name: "Aleqsd.MayhemLens".into(),
            publisher: "CN=Test Publisher".into(),
            // Deliberately distinct words catch swapped or truncated components.
            version: 0x0001_0002_0003_0004,
            is_x64: true,
        }
    }

    fn check(actual: &PackageIdentity) -> Result<()> {
        validate_identity(
            actual,
            "Aleqsd.MayhemLens",
            "CN=Test Publisher",
            [1, 2, 3, 4],
        )
    }

    #[test]
    fn accepts_only_exact_identity_version_and_x64() {
        assert!(check(&identity()).is_ok());
        let wrong = [
            PackageIdentity {
                name: "Another.Package".into(),
                ..identity()
            },
            PackageIdentity {
                publisher: "CN=Other Publisher".into(),
                ..identity()
            },
            PackageIdentity {
                version: 0x0001_0003_0002_0004,
                ..identity()
            },
            PackageIdentity {
                version: 0x0001_0002_0003_0005,
                ..identity()
            },
            PackageIdentity {
                is_x64: false,
                ..identity()
            },
        ];
        for actual in wrong {
            assert!(check(&actual).is_err());
        }
        assert!(validate_identity(&identity(), "", "CN=Test Publisher", [1, 2, 3, 4]).is_err());
        assert!(validate_identity(&identity(), "Aleqsd.MayhemLens", "", [1, 2, 3, 4]).is_err());
    }

    #[test]
    fn mismatch_errors_do_not_retain_untrusted_manifest_text() {
        let mut actual = identity();
        actual.name = "PrivatePlayer#1234 screenshot.png".into();
        let error = check(&actual).unwrap_err().to_string();
        assert_eq!(error, "Nom du package MSIX inattendu");
        assert!(!error.contains("PrivatePlayer"));
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "requires MAYHEM_PACKAGE_VALIDATION_FIXTURE pointing to our archived 1.0.0.0 MSIX"]
    fn validates_opt_in_windows_package_fixture() {
        use windows::Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize, RoUninitialize};

        struct Apartment;
        impl Drop for Apartment {
            fn drop(&mut self) {
                // SAFETY: balances the successful initialization on this test
                // thread after each validation call released all COM objects.
                unsafe { RoUninitialize() };
            }
        }

        let filename = std::env::var_os("MAYHEM_PACKAGE_VALIDATION_FIXTURE")
            .expect("set MAYHEM_PACKAGE_VALIDATION_FIXTURE explicitly for this ignored test");
        let path = Path::new(&filename);
        // SAFETY: initializes only this test thread, with balanced RAII cleanup.
        unsafe { RoInitialize(RO_INIT_MULTITHREADED) }.unwrap();
        let _apartment = Apartment;
        let name = "Aleqsd.MayhemLens";
        let publisher = "CN=Alexandre DO-O ALMEIDA";
        let version = [1, 0, 0, 0];
        // No package identity or installation is needed: the reader opens the
        // supplied archive only, and each call closes it before returning.
        validate(path, name, publisher, version).unwrap();
        assert_eq!(
            validate(path, "Other.Package", publisher, version)
                .unwrap_err()
                .to_string(),
            "Nom du package MSIX inattendu"
        );
        assert_eq!(
            validate(path, name, "CN=Other Publisher", version)
                .unwrap_err()
                .to_string(),
            "Éditeur du package MSIX inattendu"
        );
        assert_eq!(
            validate(path, name, publisher, [1, 0, 1, 0])
                .unwrap_err()
                .to_string(),
            "Version du package MSIX inattendue"
        );
    }
}
