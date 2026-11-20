//! The `rpsh` executable. Upstream: `src/powershell-unix` (`pwsh`).

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    std::process::exit(rpsh_host::run(&args));
}
