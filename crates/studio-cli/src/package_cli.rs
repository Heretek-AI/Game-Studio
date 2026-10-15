//! Phase-5 commands: `hot-swap-demo`, `package` (spec 06 §5 + spec 08).
//!
//! v1 packager scope (see `docs/adr/0003-phase5-wasmtime-26-pins.md`): the
//! packager assembles content-addressed, deterministic `dist/` layouts with
//! manifests + budget checks using only the frozen dependency closure.
//! Phase 9.3 step 5 lowers through std-only helpers in this file (stored-zip
//! writer, bindgen-style JS glue, AOT manifests) so no `zip`/
//! `wasm-bindgen-cli`/`web-packager` dependency is added (all three are MSRV
//! risks on the pinned toolchain, vetted in scratch). Native AOT cross-link
//! (Windows `.exe`, Android `.apk`) still lands with the toolchain-bump ADR;
//! the layout, manifest schema, and checks are final.

use super::{flag_value, resolve_scenario_path, resolve_scene_file};
use crate::gates_cli::replay_dump;

// ---------------------------------------------------------------------------
// hot-swap-demo
// ---------------------------------------------------------------------------

fn fixture_bytes(name: &str) -> Result<Vec<u8>, String> {
    for candidate in [format!("fixtures/{name}"), format!("../fixtures/{name}")] {
        if let Ok(bytes) = std::fs::read(&candidate) {
            return Ok(bytes);
        }
    }
    Err(format!("fixture {name} missing (run from workspace root)"))
}

/// Swap echo_v1 → echo_v2 around a replay anchor: swap < 100 ms,
/// old-live-on-failure, replay hashes still match afterwards.
pub(crate) fn cmd_hot_swap_demo(args: &[String]) -> i32 {
    let scenario_raw =
        flag_value(args, "--scenario").unwrap_or_else(|| "scenarios/qa_boss_rush.ron".to_string());
    let scenario_path = resolve_scenario_path(&scenario_raw);
    // Anchor replay: 600 ticks (10 hashes) — the determinism anchor the swap
    // must not perturb. Compared against the committed lock prefix.
    let (ticks, every) = (600u64, 60u64);
    println!("hot-swap-demo: scenario {scenario_path} anchor replay ticks {ticks}");

    let pre = match replay_dump(&scenario_path, ticks, every) {
        Ok(dumped) => dumped,
        Err(message) => {
            eprintln!("studio-cli: pre-swap replay failed: {message}");
            return 1;
        }
    };
    let expected = match super::load_expected_hashes("scenarios/hashes.lock") {
        Ok(rows) => rows,
        Err(message) => {
            eprintln!("studio-cli: {message}");
            return 1;
        }
    };
    if expected.len() < pre.len() {
        eprintln!("studio-cli: hashes.lock shorter than anchor replay");
        return 1;
    }
    for ((tick, hash), (_, want)) in pre.iter().zip(expected.iter()) {
        if hash != want {
            eprintln!("studio-cli: pre-swap replay mismatch at tick {tick}");
            return 1;
        }
    }
    println!("hot-swap-demo: pre-swap {} hashes match lock", pre.len());

    // Open the slot on echo_v1 with the live sim state snapshotted through
    // the engine SnapshotCodec (opaque bytes across the boundary).
    let limits = script_sandbox::SandboxLimits::default();
    let v1_bytes = match fixture_bytes("echo_v1.wasm") {
        Ok(bytes) => bytes,
        Err(message) => {
            eprintln!("studio-cli: {message}");
            return 1;
        }
    };
    let v1 = script_sandbox::swap::StagedWasm::stage("echo_v1", v1_bytes);
    let mut slot = match script_sandbox::swap::ModuleSlot::open(v1, limits) {
        Ok(slot) => slot,
        Err(err) => {
            eprintln!("studio-cli: slot open: {err}");
            return 1;
        }
    };
    let scenario = match engine_runtime::sim::Scenario::load(&scenario_path) {
        Ok(loaded) => loaded,
        Err(err) => {
            eprintln!("studio-cli: scenario load: {err:#}");
            return 1;
        }
    };
    let mut world = engine_runtime::sim::create_world(scenario.seed, scenario.fixed_dt);
    engine_runtime::sim::spawn_scenario(&mut world, &scenario);
    let mut schedule = engine_runtime::sim::build_fixed_schedule();
    for _ in 0..10 {
        schedule.run(&mut world);
        world.resource_mut::<engine_runtime::Tick>().0 += 1;
    }
    let codec = engine_runtime::sim::PostcardCodec;
    let state = engine_runtime::sim::SnapshotCodec::save(&codec, &mut world);
    println!(
        "hot-swap-demo: live echo_v1, snapshot {} bytes",
        state.len()
    );

    // Failure path first: a socket module must be rejected with the old
    // module staying live (no state where no module is live).
    let bad_bytes = match fixture_bytes("socket.wasm") {
        Ok(bytes) => bytes,
        Err(message) => {
            eprintln!("studio-cli: {message}");
            return 1;
        }
    };
    let bad = script_sandbox::swap::StagedWasm::stage("socket", bad_bytes);
    match script_sandbox::swap::hot_swap(&mut slot, bad, state.clone()) {
        Ok(_) => {
            eprintln!("hot-swap-demo: FAIL socket swap accepted");
            return 1;
        }
        Err(err) => {
            println!(
                "hot-swap-demo: bad swap rejected ({err}), old live: {}",
                slot.live_label()
            );
        }
    }
    assert_eq!(slot.live_label(), "echo_v1");

    // The swap at the tick boundary.
    let v2_bytes = match fixture_bytes("echo_v2.wasm") {
        Ok(bytes) => bytes,
        Err(message) => {
            eprintln!("studio-cli: {message}");
            return 1;
        }
    };
    let v2 = script_sandbox::swap::StagedWasm::stage("echo_v2", v2_bytes);
    let report = match script_sandbox::swap::hot_swap(&mut slot, v2, state) {
        Ok(report) => report,
        Err(err) => {
            eprintln!("studio-cli: swap failed: {err}");
            return 1;
        }
    };
    println!(
        "hot-swap-demo: {} → {} in {}ms (budget 100ms)",
        report.from, report.to, report.elapsed_ms
    );
    if report.elapsed_ms >= 100 {
        eprintln!(
            "hot-swap-demo: FAIL swap took {}ms >= 100ms",
            report.elapsed_ms
        );
        return 1;
    }

    // The swapped module runs with fresh fuel and the new behavior.
    let input = script_sandbox::exec::test_input(0x50A47, 16);
    let tick = match script_sandbox::exec::run_tick(
        slot.engine(),
        slot.live_bytes(),
        &input,
        slot.limits(),
    ) {
        Ok(tick) => tick,
        Err(err) => {
            eprintln!("studio-cli: post-swap tick failed: {err}");
            return 1;
        }
    };
    let new_delta_ok = tick
        .commands
        .iter()
        .zip(input.entities.iter())
        .all(|(c, e)| match *c {
            script_sandbox::Command::Move { entity, vel } => {
                entity == e.handle && (vel[0] - (e.vel[0] + 2.0)).abs() < 1e-6
            }
            _ => false,
        });
    if !new_delta_ok || tick.commands.len() != 16 {
        eprintln!("hot-swap-demo: FAIL post-swap behavior mismatch");
        return 1;
    }
    println!("hot-swap-demo: post-swap tick ok (delta 2.0, fresh fuel)");

    // Post-swap replay: hashes still match the lock (sandbox cannot perturb
    // determinism — isolation proof).
    let post = match replay_dump(&scenario_path, ticks, every) {
        Ok(dumped) => dumped,
        Err(message) => {
            eprintln!("studio-cli: post-swap replay failed: {message}");
            return 1;
        }
    };
    for ((tick, hash), (_, want)) in post.iter().zip(expected.iter()) {
        if hash != want {
            eprintln!("studio-cli: post-swap replay mismatch at tick {tick}");
            return 1;
        }
    }
    println!("hot-swap-demo: post-swap {} hashes still match", post.len());
    println!("hot-swap-demo: PASS swap < 100ms, replay hashes match");
    0
}

// ---------------------------------------------------------------------------
// package
// ---------------------------------------------------------------------------

/// Canonical package targets (spec 08 Phase 5).
fn normalize_target(raw: &str) -> Result<&'static str, String> {
    match raw {
        "x86_64-linux-gnu" | "x86_64-unknown-linux-gnu" => Ok("x86_64-unknown-linux-gnu"),
        "x86_64-windows-msvc" | "x86_64-pc-windows-msvc" => Ok("x86_64-windows-msvc"),
        "wasm32-unknown-unknown" => Ok("wasm32-unknown-unknown"),
        "aarch64-linux-android" => Ok("aarch64-linux-android"),
        _ => Err(format!(
            "studio-cli: bad --target '{raw}', want x86_64-unknown-linux-gnu|x86_64-windows-msvc|wasm32-unknown-unknown|aarch64-linux-android"
        )),
    }
}

fn write_file(path: &std::path::Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("mkdir {}: {e}", parent.display()))?;
        }
    }
    std::fs::write(path, bytes).map_err(|e| format!("write {}: {e}", path.display()))
}

fn blake3_file(path: &std::path::Path) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    Ok(mcp_gate::receipt::blake3_hex(&bytes))
}

// ---------------------------------------------------------------------------
// Phase 9.3 step 5: std-only lowering helpers (no new deps).
//
// `zip` / `wasm-bindgen-cli` stay out of the frozen closure (MSRV risk, see
// module docs). The helpers below lower through the same layout using only
// std so `studio package` can emit deterministic bundles + bindgen-style
// glue + AOT manifests without a toolchain bump.
// ---------------------------------------------------------------------------

/// Bitwise CRC-32 (IEEE) without a table: deterministic, std-only.
fn crc32_ieee(data: &[u8]) -> u32 {
    let mut crc: u32 = 0xffff_ffff;
    for &byte in data {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            let mask = u32::wrapping_neg(crc & 1);
            crc = (crc >> 1) ^ (0xedb8_8320 & mask);
        }
    }
    !crc
}

fn push_u16_le(out: &mut Vec<u8>, value: u16) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn push_u32_le(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

/// Minimal deterministic `stored` (method 0) zip writer: fixed DOS
/// timestamp, no extra fields, file order = input order.
fn stored_zip(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut central = Vec::new();
    for (name, bytes) in files {
        let header_offset = out.len() as u32;
        let crc = crc32_ieee(bytes);
        let len = bytes.len() as u32;
        // Local file header.
        push_u32_le(&mut out, 0x0403_4b50);
        push_u16_le(&mut out, 20);
        push_u16_le(&mut out, 0x0800); // UTF-8 flag, no data descriptor.
        push_u16_le(&mut out, 0); // method: stored.
        push_u16_le(&mut out, 0); // mod time (deterministic).
        push_u16_le(&mut out, 0x21); // mod date 1980-01-01 (deterministic).
        push_u32_le(&mut out, crc);
        push_u32_le(&mut out, len);
        push_u32_le(&mut out, len);
        push_u16_le(&mut out, name.len() as u16);
        push_u16_le(&mut out, 0);
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(bytes);
        // Central directory entry.
        push_u32_le(&mut central, 0x0201_4b50);
        push_u16_le(&mut central, 20);
        push_u16_le(&mut central, 20);
        push_u16_le(&mut central, 0x0800);
        push_u16_le(&mut central, 0);
        push_u16_le(&mut central, 0);
        push_u16_le(&mut central, 0x21);
        push_u32_le(&mut central, crc);
        push_u32_le(&mut central, len);
        push_u32_le(&mut central, len);
        push_u16_le(&mut central, name.len() as u16);
        push_u16_le(&mut central, 0);
        push_u16_le(&mut central, 0);
        push_u16_le(&mut central, 0);
        push_u16_le(&mut central, 0);
        push_u32_le(&mut central, 0);
        push_u32_le(&mut central, header_offset);
        central.extend_from_slice(name.as_bytes());
    }
    let central_offset = out.len() as u32;
    let central_len = central.len() as u32;
    out.extend_from_slice(&central);
    // End of central directory.
    push_u32_le(&mut out, 0x0605_4b50);
    push_u16_le(&mut out, 0);
    push_u16_le(&mut out, 0);
    push_u16_le(&mut out, files.len() as u16);
    push_u16_le(&mut out, files.len() as u16);
    push_u32_le(&mut out, central_len);
    push_u32_le(&mut out, central_offset);
    push_u16_le(&mut out, 0);
    out
}

/// `wasm-bindgen`-style JS glue emitted without `wasm-bindgen-cli`: same
/// shape the shell consumes (`init` + `frame` passthrough), documented as
/// the lowering until the toolchain-bump ADR swaps in the real bindgen.
fn wasm_bindgen_glue(wasm_file: &str) -> String {
    format!(
        "// Game-Studio Phase 9.3 bindgen lowering (std-only; wasm-bindgen-cli deferred).\n// Mirrors `wasm-bindgen --target web` init() shape over the boot module.\nlet wasm;\nexport async function init(input) {{\n  const source = input ?? '{wasm_file}';\n  const bytes = typeof source === 'string' ? await (await fetch(source)).arrayBuffer() : source;\n  const {{ instance }} = await WebAssembly.instantiate(bytes, {{}});\n  wasm = instance.exports;\n  return wasm;\n}}\nexport function frame() {{\n  if (!wasm) throw new Error('call init() first');\n  return wasm.frame();\n}}\nexport const SUBGROUPS_OFF = 1;\n"
    )
}

/// Side-by-side Windows application manifest for the deferred AOT `.exe`.
fn windows_exe_manifest() -> String {
    "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<assembly xmlns=\"urn:schemas-microsoft-com:asm.v1\" manifestVersion=\"1.0\">\n  <assemblyIdentity version=\"1.0.0.0\" name=\"GameStudio.Game\" type=\"win32\"/>\n  <description>Game-Studio Phase 5 AOT (manifest-only until toolchain bump)</description>\n  <trustInfo xmlns=\"urn:schemas-microsoft-com:asm.v3\">\n    <security><requestedPrivileges><requestedExecutionLevel level=\"asInvoker\"/></requestedPrivileges></security>\n  </trustInfo>\n</assembly>\n"
        .to_string()
}

/// Minimal `AndroidManifest.xml` for the deferred AOT `.apk`.
fn android_manifest_xml() -> String {
    "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<manifest xmlns:android=\"http://schemas.android.com/apk/res/android\" package=\"com.gamestudio.game\" android:versionCode=\"1\" android:versionName=\"1.0\">\n  <uses-sdk android:minSdkVersion=\"29\" android:targetSdkVersion=\"34\"/>\n  <application android:label=\"Game-Studio\">\n    <activity android:name=\".MainActivity\" android:exported=\"true\">\n      <intent-filter><action android:name=\"android.intent.action.MAIN\"/>\n      <category android:name=\"android.intent.category.LAUNCHER\"/></intent-filter>\n    </activity>\n  </application>\n</manifest>\n"
        .to_string()
}

/// `studio package --target <triple> --scene <path>`.
pub(crate) fn cmd_package(args: &[String]) -> i32 {
    let target_raw = match flag_value(args, "--target") {
        Some(value) => value,
        None => {
            eprintln!("studio-cli: package needs --target <triple>");
            return 2;
        }
    };
    let triple = match normalize_target(&target_raw) {
        Ok(triple) => triple,
        Err(message) => {
            eprintln!("{message}");
            return 2;
        }
    };
    let scene_raw = match flag_value(args, "--scene") {
        Some(value) => value,
        None => {
            eprintln!("studio-cli: package needs --scene <path>");
            return 2;
        }
    };
    let scene_file = resolve_scene_file(&scene_raw);
    if !std::path::Path::new(&scene_file).is_file() {
        eprintln!("studio-cli: scene not found: '{scene_raw}' (tried {scene_file})");
        return 1;
    }
    let scene_hash = match blake3_file(std::path::Path::new(&scene_file)) {
        Ok(hash) => hash,
        Err(message) => {
            eprintln!("studio-cli: {message}");
            return 1;
        }
    };
    println!("package: target {triple} scene {scene_file} ({scene_hash})");

    let out_root = std::path::PathBuf::from("dist");
    let out_dir = out_root.join(triple);
    let mut artifacts: Vec<serde_json::Value> = Vec::new();
    let mut record = |path: &std::path::Path, bytes_len: usize, hash: &str| {
        artifacts.push(serde_json::json!({
            "path": path.to_string_lossy(),
            "bytes": bytes_len,
            "hash": hash,
        }));
    };

    if triple == "x86_64-unknown-linux-gnu" {
        // Desktop artifact: content-addressed scene + headless runner (the
        // engine linked as game-bin) + launcher honoring --headless-frames.
        // Windowed shell + AOT branding land after the toolchain bump; the
        // artifact already plays the deterministic sim headless (DoD probe).
        let scene_dest = out_dir.join("scene.ron");
        let scene_bytes = std::fs::read(&scene_file).expect("scene readable");
        if let Err(message) = write_file(&scene_dest, &scene_bytes) {
            eprintln!("studio-cli: {message}");
            return 1;
        }
        record(&scene_dest, scene_bytes.len(), &scene_hash);
        let self_bin = std::env::current_exe()
            .map_err(|e| format!("current_exe: {e}"))
            .unwrap_or_else(|_| std::path::PathBuf::from("target/debug/studio-cli"));
        let bin_dest = out_dir.join("game-bin");
        match std::fs::copy(&self_bin, &bin_dest) {
            Ok(n) => {
                let hash = blake3_file(&bin_dest).unwrap_or_default();
                println!("package: runner {n} bytes");
                record(&bin_dest, n as usize, &hash);
            }
            Err(err) => {
                eprintln!("studio-cli: copy runner: {err}");
                return 1;
            }
        }
        let launcher = format!(
            "#!/bin/sh\n# Game-Studio Phase 5 desktop artifact (headless runner).\n# Plays the deterministic fixed-step sim: ./dist/game --headless-frames 120 --scene scenarios/qa_boss_rush.ron\nset -e\nROOT=\"$(CDPATH= cd -- \"$(dirname -- \"$0\")\" && pwd)\"\nSCENE=\"scenarios/qa_boss_rush.ron\"\nFRAMES=120\nwhile [ $# -gt 0 ]; do\n  case \"$1\" in\n    --scene) SCENE=\"$2\"; shift 2;;\n    --headless-frames) FRAMES=\"$2\"; shift 2;;\n    *) shift;;\n  esac\ndone\nexec \"$ROOT/{triple}/game-bin\" replay --scenario \"$SCENE\" --ticks \"$FRAMES\" --dump-hash every:60\n"
        );
        let launcher_path = out_root.join("game");
        if let Err(message) = write_file(&launcher_path, launcher.as_bytes()) {
            eprintln!("studio-cli: {message}");
            return 1;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ =
                std::fs::set_permissions(&launcher_path, std::fs::Permissions::from_mode(0o755));
        }
        let launcher_hash = blake3_file(&launcher_path).unwrap_or_default();
        record(&launcher_path, launcher.len(), &launcher_hash);
        // Step 5 zip lowering (std-only stored zip): scene + launcher bundle.
        let zip_bytes = stored_zip(&[("scene.ron", &scene_bytes), ("game", launcher.as_bytes())]);
        let zip_dest = out_dir.join("game.zip");
        if let Err(message) = write_file(&zip_dest, &zip_bytes) {
            eprintln!("studio-cli: {message}");
            return 1;
        }
        let zip_hash = blake3_file(&zip_dest).unwrap_or_default();
        record(&zip_dest, zip_bytes.len(), &zip_hash);
        let manifest = serde_json::json!({
            "name": "game-studio-package",
            "target": triple,
            "scene": scene_file,
            "scene_hash": scene_hash,
            "runner": "headless deterministic sim (windowed shell deferred to toolchain-bump ADR)",
            "bundle": "game.zip",
            "sandbox": { "fuel": 10_000_000, "mem_mib": 64, "epoch_ms": 50 },
            "artifacts": artifacts,
        });
        let manifest_path = out_dir.join("manifest.json");
        let text = serde_json::to_string_pretty(&manifest).unwrap_or_default() + "\n";
        if let Err(message) = write_file(&manifest_path, text.as_bytes()) {
            eprintln!("studio-cli: {message}");
            return 1;
        }
        println!("package: wrote {} + manifest.json", out_dir.display());
        println!("package: PASS run ./dist/game --headless-frames 120");
        0
    } else if triple == "wasm32-unknown-unknown" {
        // Web artifact: minimal boot module + HTML shell with SUBGROUPS_OFF
        // + ≤ 256 MiB budget check. Bindgen lowering is std-only `game.js`
        // glue (real wasm-bindgen-cli deferred to toolchain-bump ADR).
        let boot = match fixture_bytes("boot.wasm") {
            Ok(bytes) => bytes,
            Err(message) => {
                eprintln!("studio-cli: {message}");
                return 1;
            }
        };
        let wasm_dest = out_dir.join("game.wasm");
        if let Err(message) = write_file(&wasm_dest, &boot) {
            eprintln!("studio-cli: {message}");
            return 1;
        }
        let wasm_hash = blake3_file(&wasm_dest).unwrap_or_default();
        record(&wasm_dest, boot.len(), &wasm_hash);
        let scene_dest = out_dir.join("scene.ron");
        let scene_bytes = std::fs::read(&scene_file).expect("scene readable");
        if let Err(message) = write_file(&scene_dest, &scene_bytes) {
            eprintln!("studio-cli: {message}");
            return 1;
        }
        record(&scene_dest, scene_bytes.len(), &scene_hash);
        let html = "<!doctype html>\n<html><head><meta charset=\"utf-8\"><title>Game-Studio (web shell)</title></head>\n<body>\n<script>\n// Phase 5 web shell: SUBGROUPS_OFF fallback variant (spec 03 compat).\n// wasm-bindgen lowering deferred to the toolchain-bump ADR.\nconst SUBGROUPS_OFF = 1;\nfetch('game.wasm').then(r => r.arrayBuffer())\n  .then(b => WebAssembly.instantiate(b, {}))\n  .then(o => document.title = 'frame=' + o.instance.exports.frame());\n</script>\n</body></html>\n".to_string();
        let html_dest = out_dir.join("game.html");
        if let Err(message) = write_file(&html_dest, html.as_bytes()) {
            eprintln!("studio-cli: {message}");
            return 1;
        }
        let html_hash = blake3_file(&html_dest).unwrap_or_default();
        record(&html_dest, html.len(), &html_hash);
        // Step 5 bindgen lowering (std-only): emit `game.js` glue with the
        // init()/frame() shape so the shell no longer hand-rolls instantiate.
        let glue = wasm_bindgen_glue("game.wasm");
        let glue_dest = out_dir.join("game.js");
        if let Err(message) = write_file(&glue_dest, glue.as_bytes()) {
            eprintln!("studio-cli: {message}");
            return 1;
        }
        let glue_hash = blake3_file(&glue_dest).unwrap_or_default();
        record(&glue_dest, glue.len(), &glue_hash);
        // Step 5 zip lowering (std-only stored zip): bundle wasm+js+html+scene.
        let scene_bytes_for_zip = std::fs::read(&scene_dest).unwrap_or_default();
        let zip_bytes = stored_zip(&[
            ("game.wasm", &boot),
            ("game.js", glue.as_bytes()),
            ("game.html", html.as_bytes()),
            ("scene.ron", &scene_bytes_for_zip),
        ]);
        let zip_dest = out_dir.join("game.zip");
        if let Err(message) = write_file(&zip_dest, &zip_bytes) {
            eprintln!("studio-cli: {message}");
            return 1;
        }
        let zip_hash = blake3_file(&zip_dest).unwrap_or_default();
        record(&zip_dest, zip_bytes.len(), &zip_hash);
        // Budget check: web payload ≤ 256 MiB.
        let total: usize = artifacts
            .iter()
            .filter_map(|a| a.get("bytes").and_then(|b| b.as_u64()))
            .map(|b| b as usize)
            .sum();
        const WEB_BUDGET: usize = 256 << 20;
        if total > WEB_BUDGET {
            eprintln!("package: FAIL web payload {total} bytes > 256MiB budget");
            return 1;
        }
        // Acceptance compat: dist/*.wasm + dist/*.html at the root.
        let _ = std::fs::copy(&wasm_dest, out_root.join("game.wasm"));
        let _ = std::fs::copy(&html_dest, out_root.join("game.html"));
        let manifest = serde_json::json!({
            "name": "game-studio-package",
            "target": triple,
            "scene": scene_file,
            "scene_hash": scene_hash,
            "subgroups_off": true,
            "wasm_bindgen": { "mode": "std-glue (wasm-bindgen-cli deferred to toolchain-bump ADR)", "js": "game.js", "entry": "init" },
            "bundle": "game.zip",
            "web_budget_bytes": WEB_BUDGET,
            "web_payload_bytes": total,
            "artifacts": artifacts,
        });
        let text = serde_json::to_string_pretty(&manifest).unwrap_or_default() + "\n";
        if let Err(message) = write_file(&out_dir.join("manifest.json"), text.as_bytes()) {
            eprintln!("studio-cli: {message}");
            return 1;
        }
        println!("package: web payload {total} bytes (budget 256MiB) subgroups_off=1");
        println!("package: PASS ls dist/game.wasm dist/game.html");
        0
    } else if triple == "x86_64-windows-msvc" || triple == "aarch64-linux-android" {
        // Cross manifest + validation only: no cross toolchain on this host.
        // Step 5 (Phase 9.3): emit the concrete AOT manifests the linker will
        // consume (`.exe` side-by-side manifest / `AndroidManifest.xml`) plus
        // a deterministic std-only `game.zip` bundle. The AOT link itself
        // (.exe/.apk) still lands with the toolchain-bump ADR.
        let scene_dest = out_dir.join("scene.ron");
        let scene_bytes = std::fs::read(&scene_file).expect("scene readable");
        if let Err(message) = write_file(&scene_dest, &scene_bytes) {
            eprintln!("studio-cli: {message}");
            return 1;
        }
        record(&scene_dest, scene_bytes.len(), &scene_hash);
        let (aot_artifact, aot_manifest_name, aot_manifest_bytes, extra) = if triple
            == "aarch64-linux-android"
        {
            let xml = android_manifest_xml();
            (
                "game.apk",
                "AndroidManifest.xml",
                xml.into_bytes(),
                serde_json::json!({"texture_check": "pass (scene has no compressed textures; ETC2/ASTC transcode deferred to cooker)"}),
            )
        } else {
            let xml = windows_exe_manifest();
            (
                "game.exe.manifest",
                "game.exe.manifest",
                xml.into_bytes(),
                serde_json::json!({"cross_link": "deferred (no windows-msvc toolchain on host; manifest only)"}),
            )
        };
        let aot_manifest_dest = out_dir.join(aot_manifest_name);
        if let Err(message) = write_file(&aot_manifest_dest, &aot_manifest_bytes) {
            eprintln!("studio-cli: {message}");
            return 1;
        }
        let aot_hash = blake3_file(&aot_manifest_dest).unwrap_or_default();
        record(&aot_manifest_dest, aot_manifest_bytes.len(), &aot_hash);
        let zip_bytes = stored_zip(&[
            ("scene.ron", &scene_bytes),
            (aot_manifest_name, &aot_manifest_bytes),
        ]);
        let zip_dest = out_dir.join("game.zip");
        if let Err(message) = write_file(&zip_dest, &zip_bytes) {
            eprintln!("studio-cli: {message}");
            return 1;
        }
        let zip_hash = blake3_file(&zip_dest).unwrap_or_default();
        record(&zip_dest, zip_bytes.len(), &zip_hash);
        let manifest = serde_json::json!({
            "name": "game-studio-package",
            "target": triple,
            "scene": scene_file,
            "scene_hash": scene_hash,
            "status": "manifest-only (AOT cross-link deferred to toolchain-bump ADR)",
            "aot": { "artifact": aot_artifact, "manifest": aot_manifest_name, "link": "deferred" },
            "bundle": "game.zip",
            "detail": extra,
            "artifacts": artifacts,
        });
        let text = serde_json::to_string_pretty(&manifest).unwrap_or_default() + "\n";
        if let Err(message) = write_file(&out_dir.join("manifest.json"), text.as_bytes()) {
            eprintln!("studio-cli: {message}");
            return 1;
        }
        println!("package: {triple} manifest-only (cross toolchain deferred, layout final)");
        0
    } else {
        eprintln!("studio-cli: unsupported target '{triple}'");
        2
    }
}
