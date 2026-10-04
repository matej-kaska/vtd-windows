fn main() {
    println!("cargo:rerun-if-changed=assets/models.json");
    let catalog: serde_json::Value =
        serde_json::from_str(include_str!("assets/models.json")).unwrap();
    let mut generated = format!(
        "pub const BENCHMARK: &str = {:?};\npub static MODELS: &[Model] = &[\n",
        catalog["benchmark"].as_str().unwrap()
    );
    for model in catalog["models"].as_array().unwrap() {
        generated.push_str("Model {\n");
        for field in ["id", "name", "file", "url", "sha256", "details"] {
            generated.push_str(&format!("{field}: {:?},\n", model[field].as_str().unwrap()));
        }
        generated.push_str(&format!(
            "engine: EngineKind::{}, bytes: {}, autodetect: {}, languages: &{:?},\n}},\n",
            model["engine"].as_str().unwrap(),
            model["bytes"],
            model["autodetect"],
            model["languages"]
                .as_array()
                .unwrap()
                .iter()
                .map(|x| x.as_str().unwrap())
                .collect::<Vec<_>>()
        ));
    }
    generated.push_str("];\n");
    let out = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    std::fs::write(out.join("model_catalog.rs"), generated).unwrap();
    println!("cargo:rerun-if-changed=assets/icon.ico");
    println!("cargo:rerun-if-changed=assets/settings.rc");
    println!("cargo:rerun-if-changed=assets/vtd.manifest");
    println!("cargo:rerun-if-changed=assets/resident.manifest");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    if std::env::var_os("CARGO_FEATURE_TRANSCRIBE_ENGINE").is_some() {
        assert!(
            std::env::var_os("CARGO_FEATURE_ENGINE").is_none(),
            "Build engine and transcribe-engine separately: their GGML versions must not share a process"
        );
        println!("cargo:rerun-if-env-changed=VTD_TRANSCRIBE_LIB_DIR");
        println!("cargo:rerun-if-env-changed=VULKAN_SDK");
        let lib = std::env::var("VTD_TRANSCRIBE_LIB_DIR")
            .expect("Run scripts/build-transcribe.ps1 before building the native engine");
        println!("cargo:rustc-link-search=native={lib}");
        let sdk = std::env::var("VULKAN_SDK").expect("VULKAN_SDK is required");
        println!("cargo:rustc-link-search=native={sdk}/Lib");
        for library in [
            "vtd-transcribe-bridge",
            "transcribe",
            "ggml",
            "ggml-cpu",
            "ggml-vulkan",
            "ggml-base",
        ] {
            println!("cargo:rerun-if-changed={lib}/{library}.lib");
            println!("cargo:rustc-link-lib=static={library}");
        }
        for library in ["msvcprt", "vulkan-1", "Cabinet"] {
            println!("cargo:rustc-link-lib={library}");
        }
    }
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
    println!("cargo:rustc-link-arg-bin=vtd-helper={}", resource.display());
    let resident_script = out.join("resident.rc");
    let resident_resource = out.join("resident.res");
    let tray_icon = out.join("tray.ico");
    compact_tray_icon(&icon, &tray_icon);
    std::fs::write(
        &resident_script,
        format!(
            "1 ICON \"{}\"\n1 24 \"{assets}/resident.manifest\"\n",
            tray_icon.to_string_lossy().replace('\\', "/")
        ),
    )
    .unwrap();
    assert!(
        std::process::Command::new("rc.exe")
            .arg("/nologo")
            .arg("/fo")
            .arg(&resident_resource)
            .arg(&resident_script)
            .status()
            .expect("Windows SDK resource compiler is required")
            .success()
    );
    println!(
        "cargo:rustc-link-arg-bin=vtd={}",
        resident_resource.display()
    );
    for argument in [
        "/ENTRY:mainCRTStartup",
        "/NODEFAULTLIB",
        "/STACK:131072,4096",
    ] {
        println!("cargo:rustc-link-arg-bin=vtd={argument}");
    }
}

// Preserve original pixels for every tray size up to 64 px (400% DPI).
// The visible settings window and installer retain the full ICO.
fn compact_tray_icon(source: &std::path::Path, target: &std::path::Path) {
    let bytes = std::fs::read(source).unwrap();
    assert_eq!(&bytes[..4], &[0, 0, 1, 0]);
    let count = u16::from_le_bytes(bytes[4..6].try_into().unwrap()) as usize;
    let entries: Vec<_> = (0..count)
        .map(|index| &bytes[6 + index * 16..6 + (index + 1) * 16])
        .filter(|entry| (1..=64).contains(&entry[0]) && (1..=64).contains(&entry[1]))
        .collect();
    assert!(!entries.is_empty());
    let mut output = vec![0, 0, 1, 0];
    output.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    let mut data = Vec::new();
    for entry in &entries {
        let size = u32::from_le_bytes(entry[8..12].try_into().unwrap()) as usize;
        let start = u32::from_le_bytes(entry[12..16].try_into().unwrap()) as usize;
        output.extend_from_slice(&entry[..12]);
        output.extend_from_slice(&((6 + entries.len() * 16 + data.len()) as u32).to_le_bytes());
        data.extend_from_slice(&bytes[start..start + size]);
    }
    output.extend_from_slice(&data);
    std::fs::write(target, output).unwrap();
}
