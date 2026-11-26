import { beforeEach, describe, expect, it } from "vitest";
import { useConfirmStore } from "../../src/stores/confirm.store";

describe("confirm store", () => {
  beforeEach(() => {
    useConfirmStore.setState({ isOpen: false, resolve: null });
  });

  it("resolves a still-open confirm with false when a new one replaces it", async () => {
    const first = useConfirmStore.getState().confirm({ title: "A", message: "a" });
    const second = useConfirmStore.getState().confirm({ title: "B", message: "b", destructive: true });

    await expect(first).resolves.toBe(false);
    expect(useConfirmStore.getState().title).toBe("B");
    expect(useConfirmStore.getState().isOpen).toBe(true);

    useConfirmStore.getState().handleConfirm();
    await expect(second).resolves.toBe(true);
    expect(useConfirmStore.getState().isOpen).toBe(false);
  });

  it("resolves false on cancel", async () => {
    const pending = useConfirmStore.getState().confirm({ title: "A", message: "a" });
    useConfirmStore.getState().handleCancel();
    await expect(pending).resolves.toBe(false);
  });
});
