fn main() {
    for key in [
        "GOOGLE_CLIENT_ID",
        "GOOGLE_CLIENT_SECRET",
        "MICROSOFT_CLIENT_ID",
        "MICROSOFT_CLIENT_SECRET",
    ] {
        emit_env_from_dotenv("../.env", key);
    }
    tauri_build::try_build(attributes()).expect("failed to run tauri-build");
}

/// On Windows MSVC, embed the Common-Controls v6 manifest through the linker
/// so it applies to every binary of this crate, including the unit-test
/// executable. tauri-build's default only covers the app binary, and a test
/// binary that links the dialog code without it fails to start with
/// STATUS_ENTRYPOINT_NOT_FOUND (0xc0000139).
fn attributes() -> tauri_build::Attributes {
    let msvc = std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows")
        && std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc");
    if !msvc {
        return tauri_build::Attributes::new();
    }
    let manifest = std::path::Path::new(&std::env::var("CARGO_MANIFEST_DIR").unwrap())
        .join("windows-app-manifest.xml");
    println!("cargo:rerun-if-changed={}", manifest.display());
    println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
    println!("cargo:rustc-link-arg=/MANIFESTINPUT:{}", manifest.display());
    tauri_build::Attributes::new()
        .windows_attributes(tauri_build::WindowsAttributes::new_without_app_manifest())
}

fn emit_env_from_dotenv(path: &str, key: &str) {
    println!("cargo:rerun-if-changed={path}");
    println!("cargo:rerun-if-env-changed={key}");
    if let Ok(value) = std::env::var(key) {
        println!("cargo:rustc-env={key}={value}");
        return;
    }

    let Ok(contents) = std::fs::read_to_string(path) else {
        return;
    };
    let Some(value) = dotenv_lookup_from_str(&contents, key) else {
        return;
    };
    println!("cargo:rustc-env={key}={value}");
}

fn dotenv_lookup_from_str(contents: &str, key: &str) -> Option<String> {
    for raw_line in contents.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((name, value)) = line.split_once('=') else {
            continue;
        };
        if name.trim() != key {
            continue;
        }
        let value = value.trim();
        let unquoted = value
            .strip_prefix('"')
            .and_then(|v| v.strip_suffix('"'))
            .or_else(|| value.strip_prefix('\'').and_then(|v| v.strip_suffix('\'')))
            .unwrap_or(value);
        return Some(unquoted.to_string());
    }
    None
}
