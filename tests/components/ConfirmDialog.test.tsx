import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import ConfirmDialog from "../../src/components/ConfirmDialog";

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (_key: string, fallback?: string) => fallback ?? _key,
  }),
}));

describe("ConfirmDialog", () => {
  it("keeps focus inside the dialog and restores the previous focus on close", () => {
    const opener = document.createElement("button");
    document.body.appendChild(opener);
    opener.focus();

    const { unmount } = render(
      <ConfirmDialog
        title="Delete rule"
        message="This cannot be undone."
        onConfirm={vi.fn()}
        onCancel={vi.fn()}
      />,
    );

    const dialog = screen.getByRole("dialog");
    const cancelButton = screen.getByRole("button", { name: "Cancel" });
    const confirmButton = screen.getByRole("button", { name: "Confirm" });

    expect(dialog.getAttribute("aria-modal")).toBe("true");
    expect(document.activeElement).toBe(confirmButton);

    fireEvent.keyDown(document, { key: "Tab" });
    expect(document.activeElement).toBe(cancelButton);

    fireEvent.keyDown(document, { key: "Tab", shiftKey: true });
    expect(document.activeElement).toBe(confirmButton);

    unmount();

    expect(document.activeElement).toBe(opener);
    opener.remove();
  });

  it("uses the themed dialog surface instead of a plain white panel", () => {
    render(
      <ConfirmDialog
        title="Discard draft?"
        message="You have unsaved changes."
        onConfirm={vi.fn()}
        onCancel={vi.fn()}
      />,
    );

    // Surface colors come from the shared dialog tokens in index.css.
    const panel = screen.getByRole("dialog");

    expect(panel.classList.contains("dialog-panel")).toBe(true);
    expect(panel.parentElement?.classList.contains("overlay-backdrop")).toBe(true);
    // Portaled outside the app shell so transformed/isolated ancestors can't trap it.
    expect(panel.parentElement?.parentElement).toBe(document.body);
  });

  it("does not cancel when clicking outside the dialog panel", () => {
    const onCancel = vi.fn();

    render(
      <ConfirmDialog
        title="Discard draft?"
        message="You have unsaved changes."
        onConfirm={vi.fn()}
        onCancel={onCancel}
      />,
    );

    const dialog = screen.getByRole("dialog");
    fireEvent.mouseDown(dialog);
    fireEvent.click(dialog);

    expect(onCancel).not.toHaveBeenCalled();
  });

  it("keeps actions inert and focus contained while a destructive action is busy", () => {
    const onCancel = vi.fn();
    const onConfirm = vi.fn();
    render(
      <ConfirmDialog
        title="Discard draft?"
        message="Cleaning up the saved draft."
        confirmLabel="Discard"
        busyConfirmLabel="Discarding..."
        destructive
        busy
        onConfirm={onConfirm}
        onCancel={onCancel}
      />,
    );

    const dialog = screen.getByRole("dialog");
    const cancelButton = screen.getByRole("button", { name: "Cancel" });
    const confirmButton = screen.getByRole("button", { name: "Discarding..." });
    expect(dialog.getAttribute("aria-busy")).toBe("true");
    expect(cancelButton.getAttribute("aria-disabled")).toBe("true");
    expect(confirmButton.getAttribute("aria-disabled")).toBe("true");

    fireEvent.click(cancelButton);
    fireEvent.click(confirmButton);
    fireEvent.keyDown(document, { key: "Escape" });
    expect(onCancel).not.toHaveBeenCalled();
    expect(onConfirm).not.toHaveBeenCalled();

    fireEvent.keyDown(document, { key: "Tab" });
    expect(document.activeElement).toBe(confirmButton);
    fireEvent.keyDown(document, { key: "Tab" });
    expect(document.activeElement).toBe(cancelButton);
  });

  it("renders an error under the message and consumes Escape before global shortcuts", () => {
    const onCancel = vi.fn();
    const globalHandler = vi.fn();
    document.addEventListener("keydown", globalHandler);

    render(
      <ConfirmDialog
        title="Quit?"
        message="Scheduled sends are pending."
        error="Couldn't cancel the scheduled send."
        onConfirm={vi.fn()}
        onCancel={onCancel}
      />,
    );

    const alert = screen.getByRole("alert");
    expect(alert.textContent).toBe("Couldn't cancel the scheduled send.");
    expect(screen.getByRole("dialog").getAttribute("aria-describedby")).toContain(alert.id);

    fireEvent.keyDown(document.body, { key: "Escape" });
    expect(onCancel).toHaveBeenCalledOnce();
    expect(globalHandler).not.toHaveBeenCalled();
    document.removeEventListener("keydown", globalHandler);
  });

  it("only lets the top-most of two stacked dialogs handle Escape", () => {
    const lower = vi.fn();
    const upper = vi.fn();
    render(
      <>
        <ConfirmDialog title="Lower" message="l" onConfirm={vi.fn()} onCancel={lower} />
        <ConfirmDialog title="Upper" message="u" onConfirm={vi.fn()} onCancel={upper} />
      </>,
    );

    fireEvent.keyDown(document.body, { key: "Escape" });
    expect(upper).toHaveBeenCalledOnce();
    expect(lower).not.toHaveBeenCalled();
  });
});
