import { fireEvent, render, screen, within } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  previewWebdavBackup: vi.fn(),
  loadAutoBackupConfig: vi.fn(),
  invalidateQueries: vi.fn(),
}));

vi.mock("react-i18next", () => ({
  initReactI18next: { type: "3rdParty", init: vi.fn() },
  useTranslation: () => ({
    t: (key: string, fallback?: string | Record<string, unknown>, options?: Record<string, unknown>) => {
      let value = typeof fallback === "string" ? fallback : key;
      const values = typeof fallback === "object" ? fallback : options;
      for (const [name, replacement] of Object.entries(values ?? {})) {
        value = value.replaceAll(`{{${name}}}`, String(replacement));
      }
      return value;
    },
  }),
}));

vi.mock("@tanstack/react-query", () => ({
  useQueryClient: () => ({ invalidateQueries: mocks.invalidateQueries }),
}));

vi.mock("@/lib/api", () => ({
  testWebdavConnection: vi.fn(),
  backupToWebdav: vi.fn(),
  exportBackupFile: vi.fn(),
  importBackupFile: vi.fn(),
  previewBackupFile: vi.fn(),
  previewWebdavBackup: (...args: unknown[]) => mocks.previewWebdavBackup(...args),
  restoreFromWebdav: vi.fn(),
  saveAutoBackupConfig: vi.fn(),
  loadAutoBackupConfig: () => mocks.loadAutoBackupConfig(),
}));

import CloudSyncTab from "@/features/settings/CloudSyncTab";
import { restoreFromWebdav } from "@/lib/api";

describe("CloudSyncTab contact backups", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mocks.loadAutoBackupConfig.mockResolvedValue(null);
    mocks.previewWebdavBackup.mockResolvedValue({
      version: 2,
      exported_at: 1_700_000_000,
      account_count: 1,
      rule_count: 2,
      kanban_card_count: 3,
      kanban_note_count: 4,
      contact_count: 7,
      has_translate_config: false,
      has_encrypted_secrets: false,
      secret_account_count: 0,
      has_translate_secret: false,
      size_bytes: 2048,
    });
  });

  it("shows the contact count in the restore preview", async () => {
    render(<CloudSyncTab />);

    fireEvent.click(screen.getByRole("button", { name: "Restore Settings Backup" }));

    expect(await screen.findByText(/Contacts: 7/)).toBeTruthy();
  });

  it("explains that existing OAuth connections are preserved after private restore", async () => {
    const preview = await mocks.previewWebdavBackup();
    mocks.previewWebdavBackup.mockResolvedValue({ ...preview, has_encrypted_secrets: true, secret_account_count: 1 });
    vi.mocked(restoreFromWebdav).mockResolvedValue("Restored");
    render(<CloudSyncTab />);
    fireEvent.change(screen.getByPlaceholderText("Required for backing up or restoring secrets"), { target: { value: "test-passphrase" } });
    fireEvent.click(screen.getByRole("button", { name: "Restore Settings Backup" }));
    const dialog = await screen.findByRole("dialog");
    fireEvent.click(within(dialog).getByRole("button", { name: "Restore and overwrite" }));
    expect(await screen.findByText(/Existing OAuth connections and mailboxes with local data were preserved/)).toBeTruthy();
  });
});
