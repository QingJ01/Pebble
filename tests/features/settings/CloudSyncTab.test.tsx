import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import CloudSyncTab from "../../../src/features/settings/CloudSyncTab";
import {
  loadAutoBackupConfig,
  saveAutoBackupConfig,
} from "../../../src/lib/api";

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, fallback?: unknown) => {
      const labels: Record<string, string> = {
        "cloudSync.webdavUrl": "WebDAV URL",
        "cloudSync.username": "Username",
        "cloudSync.password": "Password",
        "cloudSync.includeSecrets": "Include account passwords, OAuth tokens, and API keys",
        "cloudSync.secretPassphrase": "Backup encryption password",
        "cloudSync.autoBackupEnable": "Enable automatic WebDAV backup",
        "cloudSync.saveAutoBackupConfig": "Save Auto-Backup Configuration",
        "cloudSync.autoBackupConfigSaved": "Automatic backup configuration saved",
        "common.saving": "Saving…",
      };
      return labels[key] ?? (() => {
        if (typeof fallback === "string") return fallback;
        const opts = (fallback ?? {}) as Record<string, unknown>;
        const template = typeof opts.defaultValue === "string" ? opts.defaultValue : key;
        return template.replace(/\{\{(\w+)\}\}/g, (_m: string, n: string) => String(opts[n] ?? ""));
      })();
    },
  }),
}));

vi.mock("@tanstack/react-query", () => ({
  useQueryClient: () => ({ invalidateQueries: vi.fn() }),
}));

vi.mock("../../../src/lib/profileStorage", () => ({
  profileLocalStorage: {
    getItem: vi.fn(() => null),
    setItem: vi.fn(),
  },
}));

vi.mock("../../../src/lib/api", () => ({
  testWebdavConnection: vi.fn(),
  backupToWebdav: vi.fn(),
  exportBackupFile: vi.fn(),
  importBackupFile: vi.fn(),
  previewBackupFile: vi.fn(),
  previewWebdavBackup: vi.fn(),
  restoreFromWebdav: vi.fn(),
  saveAutoBackupConfig: vi.fn(),
  loadAutoBackupConfig: vi.fn(),
}));

describe("CloudSyncTab automatic backup configuration", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.mocked(loadAutoBackupConfig).mockResolvedValue({
      url: "https://dav.example.com/old",
      username: "old-user",
      has_password: true,
      has_secret_passphrase: false,
      secret_passphrase_is_weak: false,
      interval_minutes: 60,
      enabled: true,
    });
    vi.mocked(saveAutoBackupConfig).mockResolvedValue(undefined);
  });

  it("saves edited credentials and secret settings only after explicit confirmation", async () => {
    render(<CloudSyncTab />);

    const url = await screen.findByLabelText("WebDAV URL");
    fireEvent.change(url, { target: { value: "https://dav.example.com/new" } });
    fireEvent.change(screen.getByLabelText("Username"), {
      target: { value: "new-user" },
    });
    fireEvent.change(screen.getByLabelText("Password"), {
      target: { value: "new-password" },
    });
    fireEvent.click(
      screen.getByRole("checkbox", {
        name: /Include account passwords, OAuth tokens, and API keys/,
      }),
    );
    fireEvent.change(screen.getByLabelText("Backup encryption password"), {
      target: { value: "new-passphrase-long" },
    });
    fireEvent.change(screen.getByRole("combobox"), {
      target: { value: "30" },
    });
    const enabledToggle = screen.getByRole("checkbox", {
      name: "Enable automatic WebDAV backup",
    });
    fireEvent.click(enabledToggle);
    fireEvent.click(enabledToggle);

    expect(saveAutoBackupConfig).not.toHaveBeenCalled();

    fireEvent.click(
      screen.getByRole("button", { name: "Save Auto-Backup Configuration" }),
    );

    await waitFor(() => {
      expect(saveAutoBackupConfig).toHaveBeenCalledWith({
        url: "https://dav.example.com/new",
        username: "new-user",
        password: "new-password",
        secret_passphrase: "new-passphrase-long",
        interval_minutes: 30,
        enabled: true,
      });
    });
    expect(
      screen.getByRole("status").textContent,
    ).toContain("Automatic backup configuration saved");
  });

  it("shows a useful error when saving the configuration fails", async () => {
    vi.mocked(saveAutoBackupConfig).mockRejectedValueOnce(
      new Error("backend unavailable"),
    );
    render(<CloudSyncTab />);

    fireEvent.click(
      await screen.findByRole("button", {
        name: "Save Auto-Backup Configuration",
      }),
    );

    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toContain("backend unavailable");
  });

  it.each(["WebDAV URL", "Username", "Password"])(
    "rejects enabled automatic backup when %s is empty",
    async (fieldLabel) => {
      vi.mocked(loadAutoBackupConfig).mockResolvedValueOnce({
        url: "https://dav.example.com/old",
        username: "old-user",
        has_password: false,
        has_secret_passphrase: false,
      secret_passphrase_is_weak: false,
        interval_minutes: 60,
        enabled: true,
      });
      render(<CloudSyncTab />);
      fireEvent.change(await screen.findByLabelText("Password"), {
        target: { value: "typed-password" },
      });

      fireEvent.change(await screen.findByLabelText(fieldLabel), {
        target: { value: "" },
      });
      fireEvent.click(
        screen.getByRole("button", {
          name: "Save Auto-Backup Configuration",
        }),
      );

      expect(saveAutoBackupConfig).not.toHaveBeenCalled();
      expect(screen.getByRole("alert").textContent).toContain(
        "Enter a WebDAV URL, username, and password before enabling automatic backup",
      );
    },
  );

  it("rejects secret backup without an encryption password", async () => {
    render(<CloudSyncTab />);

    fireEvent.click(
      await screen.findByRole("checkbox", {
        name: /Include account passwords, OAuth tokens, and API keys/,
      }),
    );
    fireEvent.click(
      screen.getByRole("button", {
        name: "Save Auto-Backup Configuration",
      }),
    );

    expect(saveAutoBackupConfig).not.toHaveBeenCalled();
    expect(screen.getByRole("alert").textContent).toContain(
      "Enter an encryption password before including secrets",
    );
  });

  it("allows an invalid persisted automatic backup configuration to be disabled", async () => {
    vi.mocked(loadAutoBackupConfig).mockResolvedValueOnce({
      url: "",
      username: "",
      has_password: false,
      has_secret_passphrase: false,
      secret_passphrase_is_weak: false,
      interval_minutes: 60,
      enabled: true,
    });
    render(<CloudSyncTab />);

    const enabledToggle = await screen.findByRole("checkbox", {
      name: "Enable automatic WebDAV backup",
    });
    expect((enabledToggle as HTMLInputElement).disabled).toBe(false);
    fireEvent.click(enabledToggle);
    fireEvent.click(
      screen.getByRole("button", {
        name: "Save Auto-Backup Configuration",
      }),
    );

    await waitFor(() => {
      expect(saveAutoBackupConfig).toHaveBeenCalledWith({
        url: "",
        username: "",
        password: "",
        secret_passphrase: null,
        interval_minutes: 60,
        enabled: false,
      });
    });
  });

  it("never shows saved secrets and keeps them when fields are left blank", async () => {
    vi.mocked(loadAutoBackupConfig).mockResolvedValueOnce({
      url: "https://dav.example.com/old",
      username: "old-user",
      has_password: true,
      has_secret_passphrase: true,
      secret_passphrase_is_weak: false,
      interval_minutes: 60,
      enabled: true,
    });
    render(<CloudSyncTab />);

    const password = (await screen.findByLabelText("Password")) as HTMLInputElement;
    const passphrase = screen.getByLabelText("Backup encryption password") as HTMLInputElement;
    expect(password.value).toBe("");
    expect(passphrase.value).toBe("");
    expect(password.placeholder).toContain("Saved");

    fireEvent.click(
      screen.getByRole("button", { name: "Save Auto-Backup Configuration" }),
    );

    await waitFor(() => {
      expect(saveAutoBackupConfig).toHaveBeenCalledWith({
        url: "https://dav.example.com/old",
        username: "old-user",
        password: "",
        secret_passphrase: "",
        interval_minutes: 60,
        enabled: true,
      });
    });
  });

  it("keeps a weak saved passphrase working and only shows a hint", async () => {
    vi.mocked(loadAutoBackupConfig).mockResolvedValueOnce({
      url: "https://dav.example.com/old",
      username: "old-user",
      has_password: true,
      has_secret_passphrase: true,
      secret_passphrase_is_weak: true,
      interval_minutes: 60,
      enabled: true,
    });
    render(<CloudSyncTab />);

    expect((await screen.findByRole("note")).textContent).toContain("shorter than 12 characters");
    fireEvent.click(
      screen.getByRole("button", { name: "Save Auto-Backup Configuration" }),
    );

    await waitFor(() => {
      expect(saveAutoBackupConfig).toHaveBeenCalledWith(
        expect.objectContaining({ secret_passphrase: "" }),
      );
    });
  });

  it("rejects a backup encryption password shorter than 12 characters", async () => {
    render(<CloudSyncTab />);

    fireEvent.click(
      await screen.findByRole("checkbox", {
        name: /Include account passwords, OAuth tokens, and API keys/,
      }),
    );
    fireEvent.change(screen.getByLabelText("Backup encryption password"), {
      target: { value: "short-pass" },
    });
    fireEvent.click(
      screen.getByRole("button", { name: "Save Auto-Backup Configuration" }),
    );

    expect(saveAutoBackupConfig).not.toHaveBeenCalled();
    expect(screen.getByRole("alert").textContent).toContain("at least 12 characters");
  });

  it("prevents duplicate configuration saves while one is in progress", async () => {
    let resolveSave!: () => void;
    vi.mocked(saveAutoBackupConfig).mockImplementationOnce(
      () => new Promise<void>((resolve) => {
        resolveSave = resolve;
      }),
    );
    render(<CloudSyncTab />);

    const saveButton = await screen.findByRole("button", {
      name: "Save Auto-Backup Configuration",
    });
    fireEvent.click(saveButton);

    const savingButton = screen.getByRole("button", { name: "Saving…" });
    expect((savingButton as HTMLButtonElement).disabled).toBe(true);
    expect(
      (screen.getByLabelText("WebDAV URL") as HTMLInputElement).disabled,
    ).toBe(true);
    fireEvent.click(savingButton);
    expect(saveAutoBackupConfig).toHaveBeenCalledTimes(1);

    resolveSave();
    await waitFor(() => {
      expect(
        (screen.getByRole("button", {
          name: "Save Auto-Backup Configuration",
        }) as HTMLButtonElement).disabled,
      ).toBe(false);
    });
  });
});
