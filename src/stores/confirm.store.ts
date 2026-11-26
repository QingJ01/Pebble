/**
 * Global confirm dialog: `await useConfirmStore.getState().confirm({ title, message, ... })`
 * resolves true/false. Calling confirm() while another confirm is still open
 * resolves the previous promise with `false` (it never dangles) and replaces it.
 */
import { create } from "zustand";

interface ConfirmState {
  isOpen: boolean;
  title: string;
  message: string;
  destructive: boolean;
  confirmLabel?: string;
  cancelLabel?: string;
  resolve: ((value: boolean) => void) | null;
  confirm: (opts: {
    title: string;
    message: string;
    destructive?: boolean;
    confirmLabel?: string;
    cancelLabel?: string;
  }) => Promise<boolean>;
  handleConfirm: () => void;
  handleCancel: () => void;
}

export const useConfirmStore = create<ConfirmState>((set, get) => ({
  isOpen: false,
  title: "",
  message: "",
  destructive: false,
  confirmLabel: undefined,
  cancelLabel: undefined,
  resolve: null,
  confirm: (opts) =>
    new Promise<boolean>((resolve) => {
      const previous = get().resolve;
      previous?.(false);
      set({
        isOpen: true,
        title: opts.title,
        message: opts.message,
        destructive: opts.destructive ?? false,
        confirmLabel: opts.confirmLabel,
        cancelLabel: opts.cancelLabel,
        resolve,
      });
    }),
  handleConfirm: () => {
    const { resolve } = get();
    resolve?.(true);
    set({ isOpen: false, resolve: null });
  },
  handleCancel: () => {
    const { resolve } = get();
    resolve?.(false);
    set({ isOpen: false, resolve: null });
  },
}));
