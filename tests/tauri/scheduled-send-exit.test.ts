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
    expect(source.indexOf("setup_application_menu(app.handle())?")).toBeGreaterThan(
      source.indexOf("send_controls::setup(app.handle())?"),
    );
  });

  it("guards explicit exits and non-background window close before destruction", () => {
    expect(source).toContain("} else if send_controls::prevent_exit(window.app_handle()) {");
    expect(source).toContain("api.prevent_close();");
    expect(source).toContain("tauri::RunEvent::ExitRequested { api, .. }");
    expect(source).toContain("if send_controls::prevent_exit(app) {");
    expect(source).toContain("api.prevent_exit();");
  });
});
