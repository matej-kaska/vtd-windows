fn main() {
    println!("cargo:rerun-if-changed=assets/icon.ico");
    println!("cargo:rerun-if-changed=assets/settings.rc");
    println!("cargo:rerun-if-changed=assets/vtd.manifest");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let out = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    let icon = std::path::PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap())
        .join("assets/icon.ico");
    let script = out.join("icon.rc");
    let resource = out.join("icon.res");
    let assets = std::path::PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap())
        .join("assets")
        .to_string_lossy()
        .replace('\\', "/");
    std::fs::write(
        &script,
        format!(
            "#pragma code_page(65001)\n1 ICON \"{}\"\n1 24 \"{assets}/vtd.manifest\"\n#include \"{assets}/settings.rc\"\n",
            icon.to_string_lossy().replace('\\', "/")
        ),
    )
    .unwrap();
    assert!(
        std::process::Command::new("rc.exe")
            .arg("/nologo")
            .arg("/fo")
            .arg(&resource)
            .arg(script)
            .status()
            .expect("Windows SDK resource compiler is required")
            .success()
    );
    println!("cargo:rustc-link-arg-bin=vtd={}", resource.display());
}
