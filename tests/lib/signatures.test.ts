import { beforeEach, describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { getSignature, setSignature } from "../../src/lib/signatures";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

const invokeMock = vi.mocked(invoke);

describe("signatures secure storage", () => {
  beforeEach(() => {
    localStorage.clear();
    invokeMock.mockReset();
  });

  it("loads signatures from backend storage and clears legacy localStorage", async () => {
    localStorage.setItem("pebble-signatures", JSON.stringify({ "account-1": "legacy" }));
    invokeMock.mockResolvedValue("secure signature");

    const signature = await getSignature("account-1");

    expect(invokeMock).toHaveBeenCalledWith("get_email_signature", { accountId: "account-1" });
    expect(signature).toBe("secure signature");
    expect(localStorage.getItem("pebble-signatures")).toBeNull();
  });

  it("saves signatures through backend storage without writing localStorage", async () => {
    invokeMock.mockResolvedValue(undefined);

    await setSignature("account-1", "Regards");

    expect(invokeMock).toHaveBeenCalledWith("set_email_signature", {
      accountId: "account-1",
      signature: "Regards",
    });
    expect(localStorage.getItem("pebble-signatures")).toBeNull();
  });
});
