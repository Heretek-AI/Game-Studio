use mcp_gate::{handle_request_line, E_METHOD_NOT_FOUND, E_PARSE, E_VERSION, SUPPORTED_METHODS};
use serde_json::Value;

const USAGE: &str = "usage: studio-cli <command> [options]\n\
    \n\
    commands:\n\
      check\n\
          Run repository gates in order: fmt, clippy, deny, dep-tree.\n\
      rpc-conformance --transport stdio\n\
          Run in-process JSON-RPC conformance cases against mcp-gate.\n\
      replay --scenario <path> --ticks <n> --dump-hash every:<k> [--expect-hashes <path>]\n\
          Run headless fixed-step replay and compare tick hashes.\n\
      mem-report --scenario <path>\n\
          Run headless soak and report frame-arena budgets.\n\
      render-test --scene <path> --frames <n> --golden <dir> [--target desktop|web|mobile] [--write-goldens]\n\
          Compile Slang shaders (Slang->WGSL->Naga + PSO cache), render headless\n\
          frames, compare last frame against golden PNG (tolerance < 0.5%).\n\
      help | --help | -h\n\
          Print this usage.\n";

const FMT_ARGS: &[&str] = &["fmt", "--check"];
const CLIPPY_ARGS: &[&str] = &[
    "clippy",
    "--workspace",
    "--all-targets",
    "--",
    "-D",
    "warnings",
];
const DENY_ARGS: &[&str] = &["deny", "check", "licenses", "bans", "sources"];
const TREE_ARGS: &[&str] = &["tree", "-p", "engine-runtime", "--prefix", "none"];

const FORBIDDEN_DEP_TOKENS: &[&str] = &["llama", "torch", "cuda", "slint", "gpl"];

fn main() {
    std::process::exit(real_main());
}

fn real_main() -> i32 {
    let args: Vec<String> = std::env::args().collect();
    let command = match args.get(1) {
        Some(first) => first.as_str(),
        None => "help",
    };
    match command {
        "check" => cmd_check(),
        "rpc-conformance" => cmd_rpc_conformance(&args[2..]),
        "replay" => cmd_replay(&args[2..]),
        "mem-report" => cmd_mem_report(&args[2..]),
        "render-test" => cmd_render_test(&args[2..]),
        "help" | "--help" | "-h" => {
            print!("{USAGE}");
            0
        }
        _ => {
            eprintln!("studio-cli: unknown command '{command}'");
            eprint!("{USAGE}");
            2
        }
    }
}

fn run(program: &str, args: &[&str]) -> (i32, String) {
    match std::process::Command::new(program).args(args).output() {
        Ok(result) => {
            let code = result.status.code().map_or(1, |code| code);
            let mut combined = String::from_utf8_lossy(&result.stdout).into_owned();
            let stderr = String::from_utf8_lossy(&result.stderr);
            if !stderr.is_empty() {
                if !combined.is_empty() && !combined.ends_with('\n') {
                    combined.push('\n');
                }
                combined.push_str(&stderr);
            }
            (code, combined)
        }
        Err(err) => (
            127,
            format!("studio-cli: failed to start '{program}': {err}"),
        ),
    }
}

fn cmd_check() -> i32 {
    let gates: &[(&str, &str, &[&str])] = &[
        ("fmt", "cargo", FMT_ARGS),
        ("clippy", "cargo", CLIPPY_ARGS),
        ("deny", "cargo", DENY_ARGS),
    ];
    for &(name, program, args) in gates {
        let (code, output) = run(program, args);
        if code != 0 {
            println!("check: {name} ... FAIL");
            if !output.trim().is_empty() {
                println!("{output}");
            }
            return code;
        }
        println!("check: {name} ... ok");
    }
    let dep_code = gate_dep_tree();
    if dep_code != 0 {
        return dep_code;
    }
    println!("check: all gates passed");
    0
}

fn find_forbidden_lines(output: &str) -> Vec<String> {
    output
        .lines()
        .filter(|line| {
            let lower = line.to_lowercase();
            FORBIDDEN_DEP_TOKENS
                .iter()
                .any(|token| lower.contains(*token))
        })
        .map(str::to_string)
        .collect()
}

fn gate_dep_tree() -> i32 {
    let (code, output) = run("cargo", TREE_ARGS);
    if code != 0 {
        println!("check: dep-tree ... FAIL");
        if !output.trim().is_empty() {
            println!("{output}");
        }
        return code;
    }
    let hits = find_forbidden_lines(&output);
    if hits.is_empty() {
        println!("check: dep-tree ... ok");
        0
    } else {
        println!("check: dep-tree ... FAIL");
        for line in &hits {
            println!("{line}");
        }
        1
    }
}

fn cmd_rpc_conformance(args: &[String]) -> i32 {
    match parse_transport(args) {
        Ok(()) => run_conformance(),
        Err(message) => {
            eprintln!("{message}");
            eprintln!("supported transports: stdio");
            eprint!("{USAGE}");
            2
        }
    }
}

fn parse_transport(args: &[String]) -> Result<(), String> {
    let mut value: Option<&str> = None;
    let mut pending_flag = false;
    for arg in args {
        let text = arg.as_str();
        if pending_flag {
            value = Some(text);
            pending_flag = false;
        } else if text == "--transport" {
            pending_flag = true;
        } else if let Some(rest) = text.strip_prefix("--transport=") {
            value = Some(rest);
        }
    }
    match value {
        Some("stdio") => Ok(()),
        Some(other) => Err(format!("studio-cli: unsupported transport '{other}'")),
        None => Err("studio-cli: missing required flag '--transport stdio'".to_string()),
    }
}

type CaseCheck = fn(&Value) -> bool;

fn run_conformance() -> i32 {
    let supported: Vec<String> = SUPPORTED_METHODS.iter().map(|m| m.to_string()).collect();
    let joined = supported.join(", ");
    println!("conformance: supported methods: {joined}");
    let cases: &[(&str, &str, CaseCheck)] = &[
        (
            "scene.query/v1",
            r#"{"jsonrpc":"2.0","id":1,"method":"scene.query/v1","params":{}}"#,
            has_result_nodes,
        ),
        (
            "scene.getNode/v1",
            r#"{"jsonrpc":"2.0","id":2,"method":"scene.getNode/v1","params":{"id":"e:1"}}"#,
            result_id_matches,
        ),
        (
            "scene.query/v9",
            r#"{"jsonrpc":"2.0","id":3,"method":"scene.query/v9","params":{}}"#,
            error_is_version,
        ),
        ("malformed", "{not json", error_is_parse),
        (
            "foo/bar/v1",
            r#"{"jsonrpc":"2.0","id":5,"method":"foo/bar/v1","params":{}}"#,
            error_is_method_not_found,
        ),
    ];
    let mut failures = 0;
    for (offset, &(name, request, check)) in cases.iter().enumerate() {
        let response = handle_request_line(request);
        let parsed = parse_response(&response);
        let passed = parsed.as_ref().is_some_and(check);
        let number = offset + 1;
        if passed {
            println!("conformance: case {number} {name} ... ok");
        } else {
            println!("conformance: case {number} {name} ... FAIL");
            failures += 1;
        }
    }
    if failures == 0 {
        println!("conformance: all cases passed");
        0
    } else {
        println!("conformance: {failures} case(s) failed");
        1
    }
}

fn parse_response(text: &str) -> Option<Value> {
    serde_json::from_str(text).ok()
}

fn has_result_nodes(response: &Value) -> bool {
    response
        .get("result")
        .and_then(|result| result.get("nodes"))
        .is_some_and(|nodes| !nodes.is_null())
}

fn result_id_matches(response: &Value) -> bool {
    response
        .get("result")
        .and_then(|result| result.get("id"))
        .and_then(Value::as_str)
        .is_some_and(|id| id == "e:1")
}

fn error_code_is(response: &Value, wanted: i64) -> bool {
    response
        .get("error")
        .and_then(|error| error.get("code"))
        .and_then(Value::as_i64)
        .is_some_and(|code| code == wanted)
}

fn error_is_parse(response: &Value) -> bool {
    error_code_is(response, i64::from(E_PARSE))
}

fn error_is_version(response: &Value) -> bool {
    error_code_is(response, i64::from(E_VERSION))
}

fn error_is_method_not_found(response: &Value) -> bool {
    error_code_is(response, i64::from(E_METHOD_NOT_FOUND))
}

fn flag_value(args: &[String], name: &str) -> Option<String> {
    let mut pending = false;
    for arg in args {
        let text = arg.as_str();
        if pending {
            return Some(text.to_string());
        }
        if text == name {
            pending = true;
        } else if let Some(rest) = text.strip_prefix(name) {
            if let Some(stripped) = rest.strip_prefix('=') {
                return Some(stripped.to_string());
            }
        }
    }
    None
}

fn resolve_scenario_path(raw: &str) -> String {
    let direct = std::path::Path::new(raw);
    if direct.exists() {
        return raw.to_string();
    }
    let with_ron = format!("{raw}.ron");
    if std::path::Path::new(&with_ron).exists() {
        return with_ron;
    }
    raw.to_string()
}

fn parse_every(spec: &str) -> Result<u64, String> {
    let rest = spec
        .strip_prefix("every:")
        .ok_or_else(|| format!("studio-cli: bad --dump-hash '{spec}', want every:<k>"))?;
    rest.parse::<u64>()
        .map_err(|_| format!("studio-cli: bad --dump-hash '{spec}', want every:<k>"))
}

fn load_expected_hashes(path: &str) -> Result<Vec<(u64, String)>, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("studio-cli: read expect-hashes {path}: {e}"))?;
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut parts = line.split_whitespace();
        let tick = parts
            .next()
            .ok_or_else(|| format!("studio-cli: bad hashes.lock line: '{line}'"))?;
        let hash = parts
            .next()
            .ok_or_else(|| format!("studio-cli: bad hashes.lock line: '{line}'"))?;
        let tick = tick
            .parse::<u64>()
            .map_err(|_| format!("studio-cli: bad tick in hashes.lock line: '{line}'"))?;
        out.push((tick, hash.to_string()));
    }
    Ok(out)
}

fn cmd_replay(args: &[String]) -> i32 {
    let scenario_raw = match flag_value(args, "--scenario") {
        Some(value) => value,
        None => {
            eprintln!("studio-cli: replay needs --scenario <path>");
            eprint!("{USAGE}");
            return 2;
        }
    };
    let scenario_path = resolve_scenario_path(&scenario_raw);
    let ticks = match flag_value(args, "--ticks") {
        Some(value) => match value.parse::<u64>() {
            Ok(parsed) => parsed,
            Err(_) => {
                eprintln!("studio-cli: bad --ticks '{value}'");
                return 2;
            }
        },
        None => 3600,
    };
    let every = match flag_value(args, "--dump-hash") {
        Some(value) => match parse_every(&value) {
            Ok(parsed) if parsed > 0 => parsed,
            Ok(_) => {
                eprintln!("studio-cli: --dump-hash interval must be > 0");
                return 2;
            }
            Err(message) => {
                eprintln!("{message}");
                return 2;
            }
        },
        None => 60,
    };
    let expect_path = flag_value(args, "--expect-hashes");
    let scenario = match engine_runtime::sim::Scenario::load(&scenario_path) {
        Ok(loaded) => loaded,
        Err(message) => {
            eprintln!("studio-cli: {message}");
            return 1;
        }
    };
    let mut world = engine_runtime::sim::create_world(scenario.seed, scenario.fixed_dt);
    engine_runtime::sim::spawn_scenario(&mut world, &scenario);
    let mut schedule = engine_runtime::sim::build_fixed_schedule();
    let mut dumped: Vec<(u64, String)> = Vec::new();
    for _ in 0..ticks {
        schedule.run(&mut world);
        world.resource_mut::<engine_runtime::Tick>().0 += 1;
        let seed = world
            .get_resource::<engine_runtime::SimSeed>()
            .map(|s| s.0)
            .unwrap_or(0);
        let tick = world
            .get_resource::<engine_runtime::Tick>()
            .map(|t| t.0)
            .unwrap_or(0);
        world.insert_resource(engine_runtime::InputWord(
            engine_runtime::sim::derive_input_word(seed, tick),
        ));
        if tick % every == 0 {
            let hash = engine_runtime::hash_tick(&mut world);
            let line = format!("blake3:{hash}");
            println!("{tick} {line}");
            dumped.push((tick, line));
        }
    }
    if let Some(path) = expect_path {
        match load_expected_hashes(&path) {
            Ok(expected) => {
                if expected.len() != dumped.len() {
                    eprintln!(
                        "studio-cli: replay hash count mismatch: got {} want {}",
                        dumped.len(),
                        expected.len()
                    );
                    return 1;
                }
                for ((got_tick, got_hash), (want_tick, want_hash)) in
                    dumped.iter().zip(expected.iter())
                {
                    if got_tick != want_tick || got_hash != want_hash {
                        eprintln!(
                            "studio-cli: replay mismatch at tick {got_tick}: got {got_hash} want {want_hash} (want tick {want_tick})"
                        );
                        return 1;
                    }
                }
                println!("replay: all {} hashes match", dumped.len());
                0
            }
            Err(message) => {
                eprintln!("{message}");
                1
            }
        }
    } else {
        0
    }
}

fn cmd_mem_report(args: &[String]) -> i32 {
    let scenario_raw = match flag_value(args, "--scenario") {
        Some(value) => value,
        None => {
            eprintln!("studio-cli: mem-report needs --scenario <path>");
            eprint!("{USAGE}");
            return 2;
        }
    };
    let scenario_path = resolve_scenario_path(&scenario_raw);
    let scenario = match engine_runtime::sim::Scenario::load(&scenario_path) {
        Ok(loaded) => loaded,
        Err(message) => {
            eprintln!("studio-cli: {message}");
            return 1;
        }
    };
    let mut world = engine_runtime::sim::create_world(scenario.seed, scenario.fixed_dt);
    engine_runtime::sim::spawn_scenario(&mut world, &scenario);
    let mut schedule = engine_runtime::sim::build_fixed_schedule();
    let ticks: u64 = 600;
    for _ in 0..ticks {
        schedule.run(&mut world);
        world.resource_mut::<engine_runtime::Tick>().0 += 1;
        let seed = world
            .get_resource::<engine_runtime::SimSeed>()
            .map(|s| s.0)
            .unwrap_or(0);
        let tick = world
            .get_resource::<engine_runtime::Tick>()
            .map(|t| t.0)
            .unwrap_or(0);
        world.insert_resource(engine_runtime::InputWord(
            engine_runtime::sim::derive_input_word(seed, tick),
        ));
        if let Some(frame) = world.get_resource::<engine_runtime::sim::FrameArena>() {
            frame.arena.reset();
        }
    }
    let (capacity, high_water, hot_mallocs) =
        match world.get_resource::<engine_runtime::sim::FrameArena>() {
            Some(frame) => (
                frame.arena.capacity(),
                frame.arena.high_water(),
                frame.hot_mallocs,
            ),
            None => {
                eprintln!("studio-cli: mem-report missing frame arena");
                return 1;
            }
        };
    let ratio = if capacity == 0 {
        0.0
    } else {
        high_water as f64 / capacity as f64 * 100.0
    };
    println!("mem-report: scenario {scenario_path}");
    println!(
        "mem-report: ticks {ticks} entities {}",
        scenario.entity_count()
    );
    println!("mem-report: frame-arena capacity {capacity} high-water {high_water} ({ratio:.4}%)");
    println!("mem-report: hot-loop mallocs {hot_mallocs}");
    let mut code = 0;
    if ratio >= 80.0 {
        eprintln!("mem-report: FAIL frame-arena HWM {ratio:.2}% >= 80%");
        code = 1;
    }
    if hot_mallocs != 0 {
        eprintln!("mem-report: FAIL hot-loop mallocs {hot_mallocs} != 0");
        code = 1;
    }
    if code == 0 {
        println!("mem-report: PASS");
    }
    code
}

fn resolve_scene_file(raw: &str) -> String {
    let candidates = [
        raw.to_string(),
        format!("assets/{raw}"),
        format!("{raw}/scene.ron"),
        format!("assets/{raw}/scene.ron"),
    ];
    for candidate in &candidates {
        if std::path::Path::new(candidate).is_file() {
            return candidate.clone();
        }
    }
    raw.to_string()
}

fn detect_render_scene(scene_path: &str) -> engine_runtime::render::RenderScene {
    use engine_runtime::render::RenderScene;
    let lowered = scene_path.to_lowercase();
    if lowered.contains("2d") || lowered.contains("platformer") || lowered.contains("sprite") {
        RenderScene::Platformer2D
    } else {
        RenderScene::SponzaLite
    }
}

fn parse_target_mask(raw: Option<String>) -> Result<engine_runtime::render::TargetMask, String> {
    use engine_runtime::render::TargetMask;
    match raw.as_deref() {
        None | Some("desktop") => Ok(TargetMask::DESKTOP),
        Some("web") => Ok(TargetMask::WEB),
        Some("mobile") => Ok(TargetMask::MOBILE),
        Some(other) => Err(format!(
            "studio-cli: bad --target '{other}', want desktop|web|mobile"
        )),
    }
}

fn cmd_render_test(args: &[String]) -> i32 {
    use engine_runtime::render::{HeadlessRenderer, ShaderPipeline, TargetMask};
    let scene_raw = match flag_value(args, "--scene") {
        Some(value) => value,
        None => {
            eprintln!("studio-cli: render-test needs --scene <path>");
            eprint!("{USAGE}");
            return 2;
        }
    };
    let frames = match flag_value(args, "--frames") {
        Some(value) => match value.parse::<u64>() {
            Ok(parsed) if parsed > 0 => parsed,
            _ => {
                eprintln!("studio-cli: bad --frames '{value}'");
                return 2;
            }
        },
        None => 60,
    };
    let golden_dir = match flag_value(args, "--golden") {
        Some(value) => value,
        None => {
            eprintln!("studio-cli: render-test needs --golden <dir>");
            eprint!("{USAGE}");
            return 2;
        }
    };
    let target = match parse_target_mask(flag_value(args, "--target")) {
        Ok(parsed) => parsed,
        Err(message) => {
            eprintln!("{message}");
            return 2;
        }
    };
    let write_goldens = args.iter().any(|arg| arg == "--write-goldens");
    let scene_file = resolve_scene_file(&scene_raw);
    if !std::path::Path::new(&scene_file).is_file() {
        eprintln!("studio-cli: scene not found: '{scene_raw}' (tried {scene_file})");
        return 1;
    }
    let scene = detect_render_scene(&scene_raw);
    println!(
        "render-test: scene {scene_raw} ({}) frames {frames}",
        scene_file
    );

    let repo_root = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let pipeline = ShaderPipeline::new(&repo_root);
    let defines: &[(&str, &str)] = if target == TargetMask::WEB {
        &[("SUBGROUPS_OFF", "1")]
    } else {
        &[]
    };
    for slang in ["shaders/pbr.slang", "shaders/sprite.slang"] {
        let path = repo_root.join(slang);
        match pipeline.compile(&path, defines) {
            Ok(compiled) => {
                println!(
                    "render-test: slang {slang} -> blake3:{}",
                    &compiled.hash[..16.min(compiled.hash.len())]
                );
            }
            Err(err) => {
                eprintln!("render-test: slang {slang} compile failed: {err}; trying last-good");
                match pipeline.last_good(&path) {
                    Some(cached) => println!(
                        "render-test: slang {slang} -> blake3:{} (last-good)",
                        &cached.hash[..16.min(cached.hash.len())]
                    ),
                    None => {
                        eprintln!("studio-cli: shader compile failed for {slang}: {err}");
                        return 1;
                    }
                }
            }
        }
    }

    let renderer = match HeadlessRenderer::new(target) {
        Ok(renderer) => renderer,
        Err(err) => {
            eprintln!("studio-cli: headless renderer init failed: {err:#}");
            eprintln!("studio-cli: hint: install software GL (tools/ci/install-sw-gl.sh) or set WGPU_BACKEND=gl");
            return 1;
        }
    };
    println!(
        "render-test: subgroups_off={} target_bits={:#x}",
        renderer.subgroups_off(),
        target.0
    );
    let mut timings = Vec::with_capacity(frames as usize);
    let mut last = None;
    for index in 0..frames {
        match renderer.render(scene, index) {
            Ok(frame) => {
                timings.push(frame.render_ms);
                last = Some(frame);
            }
            Err(err) => {
                eprintln!("studio-cli: frame {index} failed: {err:#}");
                return 1;
            }
        }
    }
    timings.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let p50 = timings[timings.len() / 2];
    let last = last.expect("frames > 0 renders a last frame");
    println!("render-test: rendered {frames} frames p50 {p50:.2}ms");
    let golden_path = std::path::Path::new(&golden_dir).join(format!("{}.png", scene.name()));
    if write_goldens || !golden_path.is_file() {
        match engine_runtime::render::backend::save_png(&last, &golden_path) {
            Ok(()) => {
                println!("render-test: golden written {}", golden_path.display());
                println!("render-test: PASS (golden recorded; commit it)");
                return 0;
            }
            Err(err) => {
                eprintln!("studio-cli: golden write failed: {err:#}");
                return 1;
            }
        }
    }
    match engine_runtime::render::backend::compare_frames(&last, &golden_path) {
        Ok(score) => {
            println!("render-test: diff {:.4}% (budget 0.5000%)", score * 100.0);
            if score < 0.005 {
                println!("render-test: PASS diff < 0.5%");
                0
            } else {
                eprintln!(
                    "render-test: FAIL diff {:.4}% >= 0.5% vs {}",
                    score * 100.0,
                    golden_path.display()
                );
                1
            }
        }
        Err(err) => {
            eprintln!("studio-cli: golden compare failed: {err:#}");
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mcp_gate::E_INVALID_PARAMS;

    #[test]
    fn error_codes_match_documented_values() {
        assert_eq!(i64::from(E_PARSE), -32700);
        assert_eq!(i64::from(E_VERSION), -32001);
        assert_eq!(i64::from(E_METHOD_NOT_FOUND), -32601);
        assert_eq!(i64::from(E_INVALID_PARAMS), -32602);
    }

    #[test]
    fn run_reports_missing_binary() {
        let (code, output) = run("studio-cli-missing-binary-xyz", &[]);
        assert!(code != 0);
        assert!(!output.is_empty());
    }

    #[test]
    fn run_captures_command_output() {
        let (code, output) = run("cargo", &["--version"]);
        assert_eq!(code, 0);
        assert!(output.contains("cargo"));
    }

    #[test]
    fn forbidden_scan_matches_case_insensitively() {
        let text = "locking Cuda Retriever 1.0\nserde 1.0\nLLAMA-UTILS 0.2\n";
        let hits = find_forbidden_lines(text);
        assert_eq!(hits.len(), 2);
    }

    #[test]
    fn transport_parser_accepts_stdio_only() {
        let split = vec!["--transport".to_string(), "stdio".to_string()];
        assert!(parse_transport(&split).is_ok());
        let joined = vec!["--transport=stdio".to_string()];
        assert!(parse_transport(&joined).is_ok());
        let other = vec!["--transport".to_string(), "ws".to_string()];
        assert!(parse_transport(&other).is_err());
        let empty: Vec<String> = Vec::new();
        assert!(parse_transport(&empty).is_err());
    }

    #[test]
    fn predicates_read_response_envelopes() {
        let query = parse_response(r#"{"jsonrpc":"2.0","id":1,"result":{"nodes":[]}}"#);
        assert!(query.as_ref().is_some_and(has_result_nodes));
        assert!(!query.as_ref().is_some_and(result_id_matches));

        let node = parse_response(r#"{"jsonrpc":"2.0","id":2,"result":{"id":"e:1"}}"#);
        assert!(node.as_ref().is_some_and(result_id_matches));

        let version_error = parse_response(
            r#"{"jsonrpc":"2.0","id":3,"error":{"code":-32001,"message":"E_VERSION"}}"#,
        );
        assert!(version_error.as_ref().is_some_and(error_is_version));
        assert!(!version_error.as_ref().is_some_and(error_is_parse));
    }
}
