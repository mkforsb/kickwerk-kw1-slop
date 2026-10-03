//! For the web build, compile `crates/worklet` to a standalone
//! `wasm32-unknown-unknown` module and place it in `OUT_DIR`, where
//! `src/audio/web.rs` embeds it with `include_bytes!`.
//!
//! A nested cargo invocation with its own target dir avoids contending for the
//! outer build's lock. The DSP is always built with the optimized `worklet`
//! profile, even for debug builds of the app, because it runs on the audio
//! thread.

use std::env;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let workspace = manifest_dir.join("../..");
    for dir in ["crates/dsp/src", "crates/worklet/src"] {
        println!("cargo:rerun-if-changed={}", workspace.join(dir).display());
    }
    println!(
        "cargo:rerun-if-changed={}",
        workspace.join("crates/worklet/Cargo.toml").display()
    );

    let is_web =
        env::var_os("CARGO_FEATURE_WEB").is_some() && env::var("CARGO_CFG_TARGET_ARCH").as_deref() == Ok("wasm32");
    if !is_web {
        return;
    }

    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let target_dir = out_dir.join("worklet-target");
    let cargo = env::var("CARGO").unwrap_or_else(|_| "cargo".into());

    let mut cmd = Command::new(cargo);
    cmd.current_dir(&workspace)
        .args([
            "build",
            "-p",
            "kickwerk-worklet",
            "--lib",
            "--target",
            "wasm32-unknown-unknown",
            "--profile",
            "worklet",
        ])
        .arg("--target-dir")
        .arg(&target_dir);
    // Don't leak the outer build's configuration (dx sets wasm-bindgen specific
    // rustflags, cargo sets profile/target overrides) into the nested build.
    for (key, _) in env::vars_os() {
        let key = key.to_string_lossy();
        if (key.starts_with("CARGO_") && key != "CARGO_HOME") || key == "RUSTFLAGS" || key == "RUSTC_WORKSPACE_WRAPPER"
        {
            cmd.env_remove(key.as_ref());
        }
    }

    let status = cmd.status().expect("failed to run cargo for kickwerk-worklet");
    assert!(status.success(), "building kickwerk-worklet for wasm32 failed");

    let built = target_dir.join("wasm32-unknown-unknown/worklet/kickwerk_worklet.wasm");
    std::fs::copy(&built, out_dir.join("kickwerk_worklet.wasm"))
        .unwrap_or_else(|e| panic!("copying {}: {e}", built.display()));
}
