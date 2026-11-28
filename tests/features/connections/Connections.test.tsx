import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import ConnectionsTab from "../../../src/features/connections/ConnectionsTab";
import PairDialog from "../../../src/features/connections/PairDialog";

vi.mock("react-i18next", () => ({
  initReactI18next: { type: "3rdParty", init: vi.fn() },
  useTranslation: () => ({
    i18n: { language: "en" },
    t: (_key: string, fallback?: string | Record<string, unknown>, vars?: Record<string, unknown>) => {
      const options = typeof fallback === "object" ? fallback : vars;
      const text = typeof fallback === "string" ? fallback : String(options?.defaultValue ?? _key);
      return text.replace(/\{\{(\w+)\}\}/g, (_, name) => String(options?.[name] ?? ""));
    },
  }),
}));
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(() => Promise.resolve(() => {})) }));

const mockInvoke = vi.mocked(invoke);
const accounts = [
  { id: "work", email: "me@work.example", provider: "imap", account_label: null },
  { id: "gm", email: "me@gmail.com", provider: "gmail", account_label: null },
];

function wrap(ui: React.ReactNode) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(<QueryClientProvider client={client}>{ui}</QueryClientProvider>);
}

describe("MCP connections", () => {
  let enabled: boolean;
  beforeEach(() => {
    enabled = true;
    mockInvoke.mockReset();
    mockInvoke.mockImplementation((command) => {
      switch (command) {
        case "mcp_get_status":
          return Promise.resolve({
            enabled, running: enabled, endpoint: "/data/mcp.sock", command: "/Applications/Pebble.app/Contents/MacOS/pebble",
            args: ["--mcp", "--endpoint", "/data/mcp.sock"], configJson: "{\"mcpServers\":{}}",
          });
        case "mcp_list_clients":
          return Promise.resolve([
            { id: "c1", name: "Claude Desktop", grants: { work: "summary" }, createdAt: 1, lastUsedAt: null, revokedAt: null },
          ]);
        case "mcp_list_audit":
          return Promise.resolve([{ id: 1, clientId: "c1", tool: "mail_search", accounts: "work", results: 3, outcome: "ok", createdAt: 2 }]);
        case "mcp_pending_pairs":
          return Promise.resolve([{ id: "p1", client: "Cursor", createdAt: 3 }]);
        case "list_accounts":
          return Promise.resolve(accounts);
        default:
          return Promise.resolve(null);
      }
    });
  });

  it("shows how to connect, the connected apps and their activity", async () => {
    wrap(<ConnectionsTab />);
    expect(await screen.findByText("/Applications/Pebble.app/Contents/MacOS/pebble --mcp --endpoint /data/mcp.sock")).toBeTruthy();
    expect(await screen.findByText("Claude Desktop · Search")).toBeTruthy();
    expect(screen.getByText(/3 results/)).toBeTruthy();

    fireEvent.click(screen.getByLabelText("Let MCP clients use Pebble"));
    await waitFor(() => expect(mockInvoke).toHaveBeenCalledWith("mcp_set_enabled", { enabled: false }));

    fireEvent.click(screen.getByRole("button", { name: "Disconnect" }));
    await waitFor(() => expect(mockInvoke).toHaveBeenCalledWith("mcp_revoke", { clientId: "c1" }));
  });

  it("changes a client's access per account", async () => {
    wrap(<ConnectionsTab />);
    fireEvent.click(await screen.findByRole("button", { name: "Change access" }));
    const select = await screen.findByRole("combobox", { name: "Access to me@work.example" });
    fireEvent.change(select, { target: { value: "body" } });
    expect(screen.getByText(/Pebble can't control what happens to it/)).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(mockInvoke).toHaveBeenCalledWith("mcp_set_grants", { clientId: "c1", grants: { work: "body" } }));
  });

  it("asks before a new app connects; Gmail can't be chosen", async () => {
    wrap(<PairDialog />);
    const dialog = await screen.findByRole("dialog");
    expect(within(dialog).getByText("Let “Cursor” use your mail?")).toBeTruthy();
    const allow = within(dialog).getByRole("button", { name: "Allow" }) as HTMLButtonElement;
    expect(allow.disabled).toBe(true);
    expect((within(dialog).getByRole("combobox", { name: "Access to me@gmail.com" }) as HTMLSelectElement).disabled).toBe(true);
    fireEvent.change(within(dialog).getByRole("combobox", { name: "Access to me@work.example" }), { target: { value: "metadata" } });
    fireEvent.click(allow);
    await waitFor(() => expect(mockInvoke).toHaveBeenCalledWith("mcp_respond_pair", { id: "p1", approve: true, grants: { work: "metadata" } }));
  });

  it("denies with no access", async () => {
    wrap(<PairDialog />);
    fireEvent.click(await screen.findByRole("button", { name: "Don't allow" }));
    await waitFor(() => expect(mockInvoke).toHaveBeenCalledWith("mcp_respond_pair", { id: "p1", approve: false, grants: {} }));
  });
});
