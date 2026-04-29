import { beforeEach, describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { deleteTemplate, listTemplates, saveTemplate } from "../../src/lib/templates";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

const invokeMock = vi.mocked(invoke);

describe("templates secure storage", () => {
  beforeEach(() => {
    localStorage.clear();
    invokeMock.mockReset();
  });

  it("loads templates from backend storage and clears legacy localStorage", async () => {
    localStorage.setItem("pebble-templates", JSON.stringify([{ id: "legacy" }]));
    invokeMock.mockResolvedValue([{ id: "template-1", name: "Intro", subject: "Hello", body: "Body", createdAt: 1 }]);

    const templates = await listTemplates();

    expect(invokeMock).toHaveBeenCalledWith("list_email_templates");
    expect(templates).toHaveLength(1);
    expect(localStorage.getItem("pebble-templates")).toBeNull();
  });

  it("saves and deletes templates through backend storage without writing localStorage", async () => {
    invokeMock
      .mockResolvedValueOnce({ id: "template-1", name: "Intro", subject: "Hello", body: "Body", createdAt: 1 })
      .mockResolvedValueOnce(undefined);

    await saveTemplate({ name: "Intro", subject: "Hello", body: "Body" });
    await deleteTemplate("template-1");

    expect(invokeMock).toHaveBeenNthCalledWith(1, "save_email_template", {
      template: { name: "Intro", subject: "Hello", body: "Body" },
    });
    expect(invokeMock).toHaveBeenNthCalledWith(2, "delete_email_template", { id: "template-1" });
    expect(localStorage.getItem("pebble-templates")).toBeNull();
  });
});
