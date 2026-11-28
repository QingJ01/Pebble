import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import AssistantSettingsTab from "../../../src/features/agent/AssistantSettingsTab";

vi.mock("react-i18next", () => ({
  initReactI18next: { type: "3rdParty", init: vi.fn() },
  useTranslation: () => ({
    t: (_key: string, fallback?: string, vars?: Record<string, unknown>) =>
      (fallback ?? _key).replace(/\{\{(\w+)\}\}/g, (_, name) => String(vars?.[name] ?? "")),
  }),
}));
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));

const mockInvoke = vi.mocked(invoke);

function wrap() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={client}>
      <AssistantSettingsTab />
    </QueryClientProvider>,
  );
}

describe("AssistantSettingsTab", () => {
  beforeEach(() => {
    mockInvoke.mockReset();
    mockInvoke.mockImplementation((command, args) => {
      switch (command) {
        case "get_agent_model":
          return Promise.resolve({ configured: true, endpoint: "https://api.openai.com", model: "gpt-4o-mini", hasApiKey: true, local: false });
        case "set_agent_model":
          return Promise.resolve({ configured: true, ...(args as object), hasApiKey: true, local: true });
        case "test_agent_model":
          return Promise.resolve({ ok: true, latencyMs: 321, message: "" });
        default:
          return Promise.resolve(null);
      }
    });
  });

  it("fills a preset, tests and saves without resending the saved key", async () => {
    wrap();
    const endpoint = await screen.findByDisplayValue("https://api.openai.com");
    expect((screen.getByLabelText("API key") as HTMLInputElement).placeholder).toBe("Saved — leave empty to keep it");
    fireEvent.click(screen.getByRole("button", { name: "Ollama" }));
    expect((endpoint as HTMLInputElement).value).toBe("http://localhost:11434");

    fireEvent.click(screen.getByRole("button", { name: "Test connection" }));
    expect(await screen.findByText("Connected · answered in 321 ms")).toBeTruthy();
    expect(mockInvoke).toHaveBeenCalledWith("test_agent_model", { endpoint: "http://localhost:11434", model: "qwen2.5:7b", apiKey: null });

    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith("set_agent_model", {
        endpoint: "http://localhost:11434", model: "qwen2.5:7b", apiKey: null, clearApiKey: null,
      }),
    );
  });

  it("warns that cloud models receive the mail you ask about", async () => {
    wrap();
    expect(await screen.findByText(/the mail it needs to answer is sent to this service/)).toBeTruthy();
  });
});
