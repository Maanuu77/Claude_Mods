use std::path::PathBuf;

fn main() {
    // Un solo manifiesto de aplicación. El GCC de MinGW enlaza por su cuenta
    // `default-manifest.o` si lo encuentra (spec `endfile`), y con el de Tauri
    // quedaban dos: `ld` avisaba (`.rsrc merge failure: multiple non-default
    // manifests`) y Windows usaba solo uno de los dos. El nuestro
    // (`windows-app-manifest.xml`) reúne lo de ambos: Common Controls v6 para los
    // diálogos nativos, `longPathAware` y `asInvoker`.
    let windows = tauri_build::WindowsAttributes::new()
        .app_manifest(include_str!("windows-app-manifest.xml"));
    println!("cargo:rerun-if-changed=windows-app-manifest.xml");

    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("gnu")
        && std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows")
    {
        suppress_mingw_default_manifest();
    }

    tauri_build::try_build(tauri_build::Attributes::new().windows_attributes(windows))
        .expect("tauri-build");
}

/// Pone delante, en la ruta de búsqueda de GCC (`-B`), un `default-manifest.o`
/// vacío: un objeto COFF x86-64 sin secciones ni símbolos (solo la cabecera de 20
/// bytes). GCC lo enlaza en lugar del de MinGW, que no aporta nada más que su
/// manifiesto.
fn suppress_mingw_default_manifest() {
    let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("OUT_DIR"));
    let dir = out.join("no-default-manifest");
    std::fs::create_dir_all(&dir).expect("carpeta del objeto vacío");
    let mut header = [0u8; 20];
    header[..2].copy_from_slice(&0x8664u16.to_le_bytes()); // IMAGE_FILE_MACHINE_AMD64
    std::fs::write(dir.join("default-manifest.o"), header).expect("objeto vacío");
    // La barra final es necesaria: `-B` es un prefijo, no una carpeta.
    println!("cargo:rustc-link-arg=-B{}/", dir.display());
}
