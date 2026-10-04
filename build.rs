use std::{env, fs, path::PathBuf, process::Command};

fn main() {
    let output = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo OUT_DIR"));
    let version = env::var("CARGO_PKG_VERSION").expect("Cargo package version");
    let numbers: Vec<_> = version
        .split('.')
        .map(|n| n.parse::<u16>().expect("Version numérique"))
        .collect();
    assert_eq!(numbers.len(), 3, "Version major.minor.patch attendue");
    let keys = [
        "MAYHEM_SETUP_MSIX_PATH",
        "MAYHEM_SETUP_CERT_PATH",
        "MAYHEM_SETUP_MSIX_SHA256",
        "MAYHEM_SETUP_CERT_SHA256",
    ];
    for key in keys {
        println!("cargo:rerun-if-env-changed={key}");
    }
    let values: Vec<_> = keys.iter().map(|key| env::var(key).ok()).collect();
    let payload = if values.iter().all(Option::is_none) {
        "pub const PAYLOAD: &[u8] = &[];\npub const PUBLIC_CERTIFICATE: &[u8] = &[];\npub const PACKAGE_SHA256: &str = \"\";\npub const CERTIFICATE_SHA256: &str = \"\";\n".to_string()
    } else {
        assert!(
            values.iter().all(Option::is_some),
            "Les quatre paramètres du package setup sont requis ensemble"
        );
        let values: Vec<_> = values.into_iter().map(Option::unwrap).collect();
        for value in &values[2..] {
            assert!(
                value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit()),
                "Digest SHA256 invalide"
            );
        }
        for path in &values[..2] {
            assert!(PathBuf::from(path).is_file(), "Payload setup absent");
            println!("cargo:rerun-if-changed={path}");
        }
        format!(
            "pub const PAYLOAD: &[u8] = include_bytes!({:?});\npub const PUBLIC_CERTIFICATE: &[u8] = include_bytes!({:?});\npub const PACKAGE_SHA256: &str = {:?};\npub const CERTIFICATE_SHA256: &str = {:?};\n",
            values[0],
            values[1],
            values[2].to_ascii_lowercase(),
            values[3].to_ascii_lowercase()
        )
    };
    fs::write(
        output.join("setup_payload.rs"),
        format!(
            "{payload}\npub const EXPECTED_PACKAGE_VERSION: [u16; 4] = [{}, {}, {}, 0];\n",
            numbers[0], numbers[1], numbers[2]
        ),
    )
    .expect("Écriture setup payload");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    for path in ["packaging/setup.ico", "packaging/setup.manifest"] {
        println!("cargo:rerun-if-changed={path}");
    }
    let repo = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let rc = output.join("setup.rc");
    let res = output.join("setup.res");
    fs::write(&rc, format!("1 ICON {:?}\n1 24 {:?}\n1 VERSIONINFO\nFILEVERSION {},{},{},0\nPRODUCTVERSION {},{},{},0\nFILEOS 0x40004\nFILETYPE 0x1\nBEGIN\n BLOCK \"StringFileInfo\"\n BEGIN\n BLOCK \"040904B0\"\n BEGIN\n VALUE \"CompanyName\", \"Alexandre DO-O ALMEIDA\\0\"\n VALUE \"FileDescription\", \"Mayhem Lens Setup\\0\"\n VALUE \"FileVersion\", \"{}\\0\"\n VALUE \"ProductName\", \"Mayhem Lens\\0\"\n VALUE \"ProductVersion\", \"{}\\0\"\n END\n END\n BLOCK \"VarFileInfo\"\n BEGIN\n VALUE \"Translation\", 0x0409, 1200\n END\nEND\n", repo.join("packaging/setup.ico").to_string_lossy(), repo.join("packaging/setup.manifest").to_string_lossy(),numbers[0],numbers[1],numbers[2],numbers[0],numbers[1],numbers[2],version,version)).expect("Écriture ressource setup");
    let sdk_root = PathBuf::from(env::var_os("ProgramFiles(x86)").expect("Windows SDK"))
        .join("Windows Kits/10/bin");
    let mut candidates: Vec<_> = fs::read_dir(sdk_root)
        .expect("Windows SDK requis pour les ressources setup")
        .filter_map(Result::ok)
        .map(|entry| entry.path().join("x64/rc.exe"))
        .filter(|path| path.is_file())
        .collect();
    candidates.sort();
    let compiler = candidates.last().expect("rc.exe absent du Windows SDK");
    let status = Command::new(compiler)
        .arg("/nologo")
        .arg("/fo")
        .arg(&res)
        .arg(&rc)
        .status()
        .expect("Exécution rc.exe");
    assert!(status.success(), "Compilation des ressources setup échouée");
    println!(
        "cargo:rustc-link-arg-bin=mayhem-lens-setup={}",
        res.display()
    );
    println!("cargo:rustc-link-arg-bin=mayhem-lens-setup=/MANIFEST:NO");
}
