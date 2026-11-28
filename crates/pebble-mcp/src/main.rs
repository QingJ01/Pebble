//! Development entry point; the shipped app runs the same bridge as
//! `Pebble --mcp --endpoint <socket>`.

fn main() {
    let args: Vec<String> = std::env::args().collect();
    std::process::exit(pebble_mcp::cli_main(&args));
}
