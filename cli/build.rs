//! Inyecta metadatos de build para trazabilidad de runtime, con UNA SOLA fuente de verdad:
//! - **Versión**: el archivo `VERSION` en la raíz del repo (no `CARGO_PKG_VERSION`).
//!   Todo el runtime (`--version`, `/version`, `/health`) reporta `MIRAI_VERSION` = ese archivo.
//! - **Build number**: `MIRAI_BUILD` = `MIRAI_BUILD_NUMBER` de CI (github.run_number); local = "dev".
//! - **Commit**: `MIRAI_GIT_SHA`. **Timestamp**: `MIRAI_BUILD_TS`.
//! Rust no tiene `CARGO_PKG_BUILD`, por eso se exponen por env de compilación aquí.
use std::process::Command;

fn main() {
    // ÚNICA fuente de verdad de la versión: el archivo VERSION en la raíz del repo.
    let cargo_ver = std::env::var("CARGO_PKG_VERSION").unwrap_or_default();
    let version = std::fs::read_to_string("../VERSION")
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|_| cargo_ver.clone());
    println!("cargo:rustc-env=MIRAI_VERSION={version}");
    // Higiene: avisar (sin bloquear) si Cargo.toml quedó desincronizado del VERSION.
    if !cargo_ver.is_empty() && version != cargo_ver {
        println!("cargo:warning=VERSION ({version}) != Cargo.toml ({cargo_ver}); sincronizalos.");
    }

    // Commit exacto sobre el que se compiló (fallback "unknown" si no hay git).
    let sha = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".to_string());
    println!("cargo:rustc-env=MIRAI_GIT_SHA={sha}");

    // Build number: CI lo inyecta (github.run_number); local = "dev".
    let build = std::env::var("MIRAI_BUILD_NUMBER").unwrap_or_else(|_| "dev".to_string());
    println!("cargo:rustc-env=MIRAI_BUILD={build}");

    // Timestamp de build: CI lo pasa; local = "unknown" (no usar now() → rebuilds no deterministas).
    let ts = std::env::var("MIRAI_BUILD_TS").unwrap_or_else(|_| "unknown".to_string());
    println!("cargo:rustc-env=MIRAI_BUILD_TS={ts}");

    println!("cargo:rerun-if-changed=../VERSION");
    println!("cargo:rerun-if-changed=../.git/HEAD");
    println!("cargo:rerun-if-env-changed=MIRAI_BUILD_NUMBER");
    println!("cargo:rerun-if-env-changed=MIRAI_BUILD_TS");
}
