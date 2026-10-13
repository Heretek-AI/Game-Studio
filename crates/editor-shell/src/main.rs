pub mod history;
pub mod inspector;
pub mod layout;
pub mod rpc;
pub mod viewport;

const USAGE: &str = "usage: editor-shell [options] [--open <scene>]\n\
    \n\
    Phase 3 editor shell (spec 04): native viewport + egui overlay + Tauri\n\
    tools behind one frozen JSON-RPC surface. On headless CI this binary\n\
    exercises the identical open pipeline without creating a window.\n\
    \n\
    options:\n\
      --open <scene>   open scene (e.g. samples/sponza_lite) and report\n\
                       interactive timing (< 3s budget, see docs/perf/phase3.md)\n\
      --rpc-stdio      serve JSON-RPC over stdio (agents/CI)\n\
      --rpc-ws [port]  serve JSON-RPC over loopback TCP (default 39002)\n\
      --help, -h       print this usage\n";

fn main() {
    std::process::exit(real_main());
}

fn real_main() -> i32 {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().skip(1).any(|a| a == "--help" || a == "-h") {
        print!("{USAGE}");
        return 0;
    }
    if let Some(open) = flag_value(&args, "--open") {
        return cmd_open(&open);
    }
    if args.iter().any(|a| a == "--rpc-stdio") {
        rpc::serve_stdio();
        return 0;
    }
    if let Some(port) = rpc_ws_port(&args) {
        match rpc::bind_loopback(port) {
            Ok(listener) => {
                println!("editor-shell: rpc ws on 127.0.0.1:{port}");
                rpc::serve_tcp(listener);
                0
            }
            Err(err) => {
                eprintln!("editor-shell: bind 127.0.0.1:{port}: {err:#}");
                1
            }
        }
    } else {
        println!("Game-Studio editor-shell 0.1.0 — mode: shell/viewport (headless ok)");
        println!("argv: {}", args[1..].join(" "));
        0
    }
}

fn flag_value(args: &[String], name: &str) -> Option<String> {
    let mut pending = false;
    for arg in args {
        if pending {
            return Some(arg.clone());
        }
        if arg == name {
            pending = true;
        } else if let Some(rest) = arg.strip_prefix(name) {
            if let Some(stripped) = rest.strip_prefix('=') {
                return Some(stripped.to_string());
            }
        }
    }
    None
}

fn rpc_ws_port(args: &[String]) -> Option<u16> {
    let mut iter = args.iter().peekable();
    while let Some(arg) = iter.next() {
        if arg == "--rpc-ws" {
            match iter.peek() {
                Some(next) if !next.starts_with("--") => {
                    return next.parse::<u16>().ok();
                }
                _ => return Some(rpc::WS_PORT),
            }
        } else if let Some(rest) = arg.strip_prefix("--rpc-ws=") {
            return rest.parse::<u16>().ok();
        }
    }
    None
}

fn cmd_open(raw: &str) -> i32 {
    match viewport::open_scene(raw) {
        Ok(stats) => {
            println!(
                "editor-shell: opened {} ({}) entities {}",
                stats.scene, stats.scene_file, stats.entities
            );
            println!("editor-shell: load {:.1}ms", stats.load_ms);
            println!(
                "editor-shell: interactive {:.1}ms (budget 3000ms)",
                stats.interactive_ms
            );
            println!("editor-shell: mode {}", stats.mode);
            if stats.interactive_ms < 3000.0 {
                println!("editor-shell: OPEN PASS (< 3s)");
                0
            } else {
                eprintln!(
                    "editor-shell: OPEN FAIL interactive {:.1}ms >= 3000ms",
                    stats.interactive_ms
                );
                1
            }
        }
        Err(err) => {
            eprintln!("editor-shell: {err:#}");
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::rpc_ws_port;

    #[test]
    fn ws_port_parses() {
        let args = ["editor-shell".to_string(), "--rpc-ws".to_string()];
        assert_eq!(rpc_ws_port(&args), Some(39002));
        let args = [
            "editor-shell".to_string(),
            "--rpc-ws".to_string(),
            "39100".to_string(),
        ];
        assert_eq!(rpc_ws_port(&args), Some(39100));
        let args = ["editor-shell".to_string()];
        assert_eq!(rpc_ws_port(&args), None);
    }
}
