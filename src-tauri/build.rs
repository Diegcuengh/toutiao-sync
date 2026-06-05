use std::{
    env, fs, io,
    path::{Path, PathBuf},
};

use chrono::Local;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BuildState {
    major: u32,
    minor: u32,
    patch: u32,
    build_date: String,
    build_time: String,
}

#[derive(Debug, Clone)]
struct BuildInfo {
    version: String,
    build_date: String,
    build_time: String,
    build_label: String,
    app_title: String,
}

fn main() {
    let manifest_dir =
        PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("missing CARGO_MANIFEST_DIR"));
    let workspace_dir = manifest_dir
        .parent()
        .expect("src-tauri should have workspace parent")
        .to_path_buf();
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("missing OUT_DIR"));

    let build_info = prepare_build_info(&workspace_dir, &out_dir)
        .unwrap_or_else(|error| panic!("failed to prepare build info: {error}"));

    if let Err(error) = sync_embedded_runtime(&manifest_dir) {
        panic!("failed to prepare embedded-sync-runtime: {error}");
    }
    if let Err(error) = copy_debug_dist(&manifest_dir) {
        panic!("failed to prepare debug embedded-dist: {error}");
    }

    tauri_build::build();

    if let Err(error) = copy_debug_runtime(&manifest_dir) {
        panic!("failed to prepare debug sync-runtime: {error}");
    }

    println!("cargo:warning=build {}", build_info.build_label);
}

fn prepare_build_info(workspace_dir: &Path, out_dir: &Path) -> io::Result<BuildInfo> {
    let state_path = workspace_dir.join(".build-state.json");
    println!("cargo:rerun-if-changed={}", state_path.display());

    let previous = read_state(&state_path)?;
    let current = next_state(previous);
    fs::write(&state_path, serde_json::to_vec_pretty(&current)?)?;

    let build_info = BuildInfo {
        version: format!("v{}.{}.{}", current.major, current.minor, current.patch),
        build_date: current.build_date.clone(),
        build_time: current.build_time.clone(),
        build_label: format!(
            "v{}.{}.{}/{}/{}",
            current.major, current.minor, current.patch, current.build_date, current.build_time
        ),
        app_title: format!(
            "\u{4eca}\u{65e5}\u{5934}\u{6761}\u{6536}\u{85cf}/\u{559c}\u{6b22}\u{540c}\u{6b65} v{}.{}.{}/{}/{}",
            current.major, current.minor, current.patch, current.build_date, current.build_time
        ),
    };

    let generated_rust = format!(
        "pub const VERSION: &str = {:?};\n\
         pub const BUILD_DATE: &str = {:?};\n\
         pub const BUILD_TIME: &str = {:?};\n\
         pub const BUILD_LABEL: &str = {:?};\n\
         pub const APP_TITLE: &str = {:?};\n",
        build_info.version,
        build_info.build_date,
        build_info.build_time,
        build_info.build_label,
        build_info.app_title
    );
    fs::write(out_dir.join("build_info.rs"), generated_rust)?;

    let runtime_build_info = serde_json::json!({
        "version": build_info.version,
        "buildDate": build_info.build_date,
        "buildTime": build_info.build_time,
        "buildLabel": build_info.build_label,
        "appTitle": build_info.app_title,
    });
    let runtime_build_info_path = workspace_dir.join("sync-runtime").join("build-info.json");
    if let Some(parent) = runtime_build_info_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(runtime_build_info_path, serde_json::to_vec_pretty(&runtime_build_info)?)?;

    Ok(build_info)
}

fn read_state(state_path: &Path) -> io::Result<Option<BuildState>> {
    if !state_path.exists() {
        return Ok(None);
    }
    let content = fs::read_to_string(state_path)?;
    let state = serde_json::from_str::<BuildState>(&content)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    Ok(Some(state))
}

fn next_state(previous: Option<BuildState>) -> BuildState {
    let now = Local::now();
    let mut state = previous.unwrap_or(BuildState {
        major: 0,
        minor: 1,
        patch: 0,
        build_date: String::new(),
        build_time: String::new(),
    });

    state.patch += 1;
    if state.patch >= 10 {
        state.patch = 0;
        state.minor += 1;
    }
    if state.minor >= 10 {
        state.minor = 0;
        state.major += 1;
    }

    state.build_date = now.format("%Y-%m-%d").to_string();
    state.build_time = now.format("%H:%M:%S").to_string();
    state
}

fn sync_embedded_runtime(manifest_dir: &Path) -> io::Result<()> {
    let source = manifest_dir
        .parent()
        .expect("src-tauri should have workspace parent")
        .join("sync-runtime");
    let target = manifest_dir.join("embedded-sync-runtime");
    let icon_path = manifest_dir.join("icons").join("icon.ico");
    let icon_png_dir = manifest_dir.join("icons").join("png-sizes");

    println!("cargo:rerun-if-changed={}", source.display());
    println!("cargo:rerun-if-changed={}", icon_path.display());
    if icon_png_dir.exists() {
        println!("cargo:rerun-if-changed={}", icon_png_dir.display());
    }

    if target.exists() {
        fs::remove_dir_all(&target)?;
    }
    fs::create_dir_all(&target)?;
    if !source.exists() {
        return Ok(());
    }

    let source_scripts = source.join("scripts");
    if source_scripts.exists() {
        copy_dir_recursive(&source_scripts, &target.join("scripts"))?;
    }

    let source_build_info = source.join("build-info.json");
    if source_build_info.exists() {
        fs::copy(source_build_info, target.join("build-info.json"))?;
    }

    Ok(())
}

fn copy_debug_runtime(manifest_dir: &Path) -> io::Result<()> {
    let profile = env::var("PROFILE").expect("missing PROFILE");
    let source = manifest_dir
        .parent()
        .expect("src-tauri should have workspace parent")
        .join("sync-runtime");
    let target = manifest_dir.join("target").join(profile).join("sync-runtime");

    if !source.exists() {
        return Ok(());
    }

    if target.exists() {
        fs::remove_dir_all(&target)?;
    }

    copy_dir_recursive(&source, &target)
}

fn copy_debug_dist(manifest_dir: &Path) -> io::Result<()> {
    let profile = env::var("PROFILE").expect("missing PROFILE");
    let source = manifest_dir
        .parent()
        .expect("src-tauri should have workspace parent")
        .join("dist");
    let target = manifest_dir.join("target").join(profile).join("embedded-dist");

    println!("cargo:rerun-if-changed={}", source.display());

    if target.exists() {
        fs::remove_dir_all(&target)?;
    }
    if !source.exists() {
        fs::create_dir_all(&target)?;
        return Ok(());
    }

    copy_dir_recursive(&source, &target)
}

fn copy_dir_recursive(source: &Path, target: &Path) -> io::Result<()> {
    fs::create_dir_all(target)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let source_path = entry.path();
        let target_path = target.join(entry.file_name());
        let metadata = entry.metadata()?;
        if metadata.is_dir() {
            copy_dir_recursive(&source_path, &target_path)?;
        } else if metadata.is_file() {
            if let Some(parent) = target_path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::copy(&source_path, &target_path)?;
        }
    }
    Ok(())
}
