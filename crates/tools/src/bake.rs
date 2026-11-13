//! Builds the scenes of the built-in games.
//!
//! Each game's world is described by a Lua builder script in
//! `tools/scenes/<id>.lua`, written against the same API game scripts use.
//! Baking runs the builder in a headless runtime and saves the resulting
//! instance tree to `games/<id>/scenes/main.scene.json`, so the scene can be
//! opened and edited in the studio like any other.
//!
//! ```text
//! cargo run -p araria-tools --bin araria-bake            # every game
//! cargo run -p araria-tools --bin araria-bake -- obby    # one game
//! cargo run -p araria-tools --bin araria-bake -- --check # also run each game for a few seconds
//! ```

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;

use araria_core::assets::AssetManifest;
use araria_core::{DataModel, scene};
use araria_runtime::{LogLevel, ProjectFiles, Runtime};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn assets() -> Arc<AssetManifest> {
    let text = std::fs::read_to_string(repo_root().join("assets/manifest.json")).unwrap_or_default();
    Arc::new(AssetManifest::parse(&text).unwrap_or_default())
}

/// Prints script output; returns how many errors it contained.
fn report(runtime: &mut Runtime, label: &str) -> usize {
    let mut errors = 0;
    for entry in runtime.scripts.take_log() {
        let level = match entry.level {
            LogLevel::Info => "info",
            LogLevel::Warn => "warn",
            LogLevel::Error => {
                errors += 1;
                "ERROR"
            }
        };
        println!("  [{label}] {level}: {}", entry.text);
    }
    errors
}

fn bake(id: &str) -> Result<DataModel, String> {
    let builders = repo_root().join("tools/scenes");
    let source = std::fs::read_to_string(builders.join(format!("{id}.lua"))).map_err(|e| format!("tools/scenes/{id}.lua: {e}"))?;
    // Builders can `require` shared helpers from tools/scenes.
    let dir = builders.clone();
    let files = ProjectFiles::new(move |path| std::fs::read_to_string(dir.join(path.trim_start_matches("scripts/"))).ok(), |_| None);
    let manifest = assets();
    let mut runtime = Runtime::with_files(DataModel::new(), manifest.clone(), files);
    let result = runtime.scripts.run(&source, &format!("{id}.lua"));
    let errors = report(&mut runtime, id);
    result?;
    if errors > 0 {
        return Err(format!("{errors} error(s) while building"));
    }
    let mut dm = runtime.dm.borrow().clone();
    // Builder bookkeeping must not end up in the saved scene.
    let ids: Vec<_> = dm.instances().map(|i| i.id).collect();
    for instance in ids {
        dm.set_creator(instance, 0);
    }
    for instance in dm.instances() {
        if instance.is_a("MeshPart") && !manifest.models.contains_key(instance.str("Asset")) {
            return Err(format!("{} uses unknown model '{}'", dm.path(instance.id), instance.str("Asset")));
        }
        if instance.class_name() == "Sound" && !instance.str("Asset").is_empty() && !manifest.audio.contains_key(instance.str("Asset")) {
            return Err(format!("{} uses unknown sound '{}'", dm.path(instance.id), instance.str("Asset")));
        }
    }
    Ok(dm)
}

/// Plays a baked game headlessly for a while and reports script errors.
fn check(id: &str, dm: DataModel, seconds: f32) -> Result<(), String> {
    let game = repo_root().join("games").join(id);
    let files = ProjectFiles::new(move |path| std::fs::read_to_string(game.join(path)).ok(), |_| Some(1));
    let mut runtime = Runtime::with_files(dm, assets(), files);
    runtime.start();
    let mut errors = 0;
    for _ in 0..(seconds * 60.0) as usize {
        runtime.step(1.0 / 60.0);
        runtime.end_frame();
        errors += report(&mut runtime, id);
    }
    let scripts = runtime.scripts.scripts();
    for script in &scripts {
        if let Some(error) = &script.error {
            println!("  [{id}] script {} failed: {error}", script.name);
            errors += 1;
        }
    }
    if errors > 0 { Err(format!("{errors} script error(s) while playing")) } else { Ok(()) }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let run_check = args.iter().any(|a| a == "--check");
    let mut ids: Vec<String> = args.into_iter().filter(|a| !a.starts_with("--")).collect();
    if ids.is_empty() {
        let Ok(entries) = std::fs::read_dir(repo_root().join("tools/scenes")) else {
            eprintln!("tools/scenes is missing");
            return ExitCode::FAILURE;
        };
        ids = entries
            .filter_map(Result::ok)
            .filter_map(|e| e.file_name().to_str().and_then(|n| n.strip_suffix(".lua")).map(str::to_string))
            .filter(|id| repo_root().join("games").join(id).is_dir())
            .collect();
        ids.sort();
    }
    let mut failed = false;
    for id in &ids {
        let outcome = bake(id).and_then(|dm| {
            let path = repo_root().join("games").join(id).join("scenes/main.scene.json");
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            std::fs::write(&path, scene::save_scene_string(&dm) + "\n").map_err(|e| format!("{}: {e}", path.display()))?;
            let count = dm.len();
            if run_check {
                check(id, dm, 6.0)?;
            }
            Ok(count)
        });
        match outcome {
            Ok(count) => println!("{id}: {count} instances{}", if run_check { ", scripts ran cleanly" } else { "" }),
            Err(e) => {
                eprintln!("{id}: FAILED — {e}");
                failed = true;
            }
        }
    }
    if failed { ExitCode::FAILURE } else { ExitCode::SUCCESS }
}
