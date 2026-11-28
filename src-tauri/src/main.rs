#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // `Pebble --mcp --endpoint <socket>`: the local MCP bridge an MCP client
    // starts (#107, M4). It only forwards to the running app; no window,
    // no database.
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|arg| arg == "--mcp") {
        std::process::exit(pebble_mcp::cli_main(&args));
    }
    configure_linux_appimage_webkit_runtime();
    pebble_lib::run();
}

fn configure_linux_appimage_webkit_runtime() {
    #[cfg(target_os = "linux")]
    {
        // Some WebKitGTK/AppImage/Wayland combinations miss repaint updates unless accelerated
        // compositing is disabled before WebKit initializes.
        if std::env::var_os("APPIMAGE").is_some()
            && std::env::var_os("WEBKIT_DISABLE_COMPOSITING_MODE").is_none()
        {
            std::env::set_var("WEBKIT_DISABLE_COMPOSITING_MODE", "1");
        }
    }
}
