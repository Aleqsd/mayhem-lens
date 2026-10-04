#![cfg_attr(windows, windows_subsystem = "windows")]

fn main() {
    #[cfg(windows)]
    let result = execute();
    #[cfg(not(windows))]
    let result: anyhow::Result<()> = Err(anyhow::anyhow!("L'installateur nécessite Windows."));
    if let Err(error) = result {
        eprintln!("Mayhem Lens — installation : {error:#}");
        std::process::exit(1);
    }
}

#[cfg(windows)]
fn execute() -> anyhow::Result<()> {
    use mayhem_lens::installer::{engine, ui};
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    match args.as_slice() {
        [] => ui::run(),
        [arg] if arg == "--approve-certificate" => engine::approve_certificate_helper(),
        [arg] if arg == "--verify-payload" => {
            engine::verify_payload()?;
            println!(
                "Payload Mayhem Lens {} vérifié ; aucune installation effectuée.",
                env!("CARGO_PKG_VERSION")
            );
            Ok(())
        }
        [arg, path] if arg == "--render-preview" => ui::render_preview(std::path::Path::new(path)),
        [arg] if arg == "--version" => {
            println!("Mayhem Lens Setup {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        _ => anyhow::bail!("Arguments d'installation non pris en charge."),
    }
}
