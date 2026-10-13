const USAGE: &str = "usage: editor-shell [options] [--open <scene>]\n\
    \n\
    Phase 0 headless shell: prints the mode banner and exits 0.\n\
    No window is created in this phase.\n\
    \n\
    options:\n\
      --help, -h    print this usage\n";

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args
        .iter()
        .skip(1)
        .any(|arg| arg == "--help" || arg == "-h")
    {
        print!("{USAGE}");
        return;
    }
    println!("Game-Studio editor-shell 0.1.0 — mode: shell/viewport (headless ok)");
    if args.len() > 1 {
        let rest = args[1..].join(" ");
        println!("argv: {rest}");
    } else {
        println!("argv: (none)");
    }
}
