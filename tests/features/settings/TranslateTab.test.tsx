import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import TranslateTab from "../../../src/features/settings/TranslateTab";
import { getTranslateConfig, saveTranslateConfig } from "../../../src/lib/api";

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, fallback?: string) => {
      const labels: Record<string, string> = {
        "translate.apiKey": "API Key",
        "translate.savedApiKeyPlaceholder": "Saved — leave blank to keep",
        "common.save": "Save",
      };
      return labels[key] ?? (typeof fallback === "string" ? fallback : key);
    },
  }),
}));

vi.mock("@/stores/toast.store", () => ({
  useToastStore: { getState: () => ({ addToast: vi.fn() }) },
}));

vi.mock("../../../src/lib/api", () => ({
  getTranslateConfig: vi.fn(),
  saveTranslateConfig: vi.fn(),
  testTranslateConnection: vi.fn(),
}));

describe("TranslateTab saved API key", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.mocked(getTranslateConfig).mockResolvedValue({
      id: "active",
      provider_type: "llm",
      config: JSON.stringify({
        type: "llm",
        endpoint: "https://api.example.com/v1",
        api_key: "",
        has_api_key: true,
        model: "gpt-4o-mini",
        mode: "completions",
      }),
      is_enabled: true,
      created_at: 1,
      updated_at: 1,
    });
    vi.mocked(saveTranslateConfig).mockResolvedValue(undefined);
  });

  it("shows a saved-key placeholder and saves a blank key to keep it", async () => {
    render(<TranslateTab />);

    const apiKey = (await screen.findByLabelText("API Key")) as HTMLInputElement;
    await waitFor(() => expect(apiKey.placeholder).toBe("Saved — leave blank to keep"));
    expect(apiKey.value).toBe("");

    fireEvent.click(screen.getByRole("button", { name: "Save" }));

    await waitFor(() => expect(saveTranslateConfig).toHaveBeenCalledTimes(1));
    const [provider, config] = vi.mocked(saveTranslateConfig).mock.calls[0];
    expect(provider).toBe("llm");
    expect(JSON.parse(config)).toMatchObject({
      type: "llm",
      endpoint: "https://api.example.com/v1",
      api_key: "",
    });
  });
});
