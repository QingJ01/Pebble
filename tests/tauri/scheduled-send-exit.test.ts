import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

describe("scheduled-send native exit wiring", () => {
  const source = readFileSync(resolve(process.cwd(), "src-tauri/src/lib.rs"), "utf8");

  it("routes macOS menu Quit and Cmd-Q through the cancellable application exit", () => {
    const menu = source.slice(source.indexOf("fn setup_application_menu("), source.indexOf("fn setup_tray("));
    expect(menu).toContain("Menu::default(app)?");
    expect(menu).toContain("MenuItem::with_id(");
    expect(menu).toContain("APPLICATION_QUIT_ID");
    expect(menu).toContain('Some("CmdOrCtrl+Q")');
    expect(menu).not.toContain("PredefinedMenuItem::quit(");
    expect(menu).toContain("app.on_menu_event(");
    expect(menu).toContain("app.exit(0)");
    // The guarded menu goes in before any database work (so Cmd-Q is
    // cancellable during a background upgrade); send controls follow once
    // the database is open.
    const menuSetup = source.indexOf("setup_application_menu(app.handle())?;");
    expect(menuSetup).toBeGreaterThan(-1);
    expect(menuSetup).toBeLessThan(source.indexOf("pending_migration_from(&db_path)"));
    expect(source.indexOf("    send_controls::setup(app)?;")).toBeGreaterThan(-1);
  });

  it("guards explicit exits and non-background window close before destruction", () => {
    expect(source).toContain("} else if send_controls::prevent_exit(window.app_handle()) {");
    expect(source).toContain("api.prevent_close();");
    expect(source).toContain("tauri::RunEvent::ExitRequested { api, .. }");
    expect(source).toContain("if send_controls::prevent_exit(app) {");
    expect(source).toContain("api.prevent_exit();");
  });
});

describe("startup with a background database upgrade", () => {
  const lib = readFileSync(resolve(process.cwd(), "src-tauri/src/lib.rs"), "utf8");
  const sendControls = readFileSync(resolve(process.cwd(), "src-tauri/src/send_controls.rs"), "utf8");

  it("lets a failed startup quit even though send controls never came up", () => {
    const fn = sendControls.slice(sendControls.indexOf("pub fn prevent_exit("));
    expect(fn.slice(0, 400)).toContain("return !crate::startup::has_failed(app);");
  });

  it("queues mailto links before the database upgrade can start", () => {
    const listen = lib.indexOf("app.listen(DEEP_LINK_NEW_URL_EVENT");
    expect(listen).toBeGreaterThan(-1);
    expect(listen).toBeLessThan(lib.indexOf("pending_migration_from(&db_path)"));
    expect(lib.indexOf("app.manage(PendingMailtoUrls::default());")).toBeLessThan(listen);
  });
});
