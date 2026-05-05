import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, renderHook, waitFor } from "@testing-library/react";
import type { ReactNode } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { updateMessageFlags } from "@/lib/api";
import { useUpdateFlagsMutation } from "@/hooks/mutations/useUpdateFlagsMutation";

vi.mock("@/lib/api", () => ({
  updateMessageFlags: vi.fn(() => Promise.resolve()),
}));

const mockUpdateMessageFlags = vi.mocked(updateMessageFlags);

function createWrapper(queryClient: QueryClient) {
  return ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
  );
}

function createQueryClient() {
  return new QueryClient({
    defaultOptions: {
      mutations: { retry: false },
      queries: { retry: false },
    },
  });
}

describe("useUpdateFlagsMutation", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("refreshes folder unread counts after a successful read-state change", async () => {
    const queryClient = createQueryClient();
    const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");
    const { result } = renderHook(() => useUpdateFlagsMutation(), {
      wrapper: createWrapper(queryClient),
    });

    await act(async () => {
      await result.current.mutateAsync({
        messageId: "message-1",
        isRead: true,
      });
    });

    expect(mockUpdateMessageFlags).toHaveBeenCalledWith("message-1", true, undefined);
    await waitFor(() => {
      expect(invalidateSpy).toHaveBeenCalledWith({ queryKey: ["folder-unread-counts"] });
    });
  });

  it("does not refresh folder unread counts for star-only changes", async () => {
    const queryClient = createQueryClient();
    const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");
    const { result } = renderHook(() => useUpdateFlagsMutation(), {
      wrapper: createWrapper(queryClient),
    });

    await act(async () => {
      await result.current.mutateAsync({
        messageId: "message-1",
        isStarred: true,
      });
    });

    expect(mockUpdateMessageFlags).toHaveBeenCalledWith("message-1", undefined, true);
    expect(invalidateSpy).not.toHaveBeenCalledWith({ queryKey: ["folder-unread-counts"] });
  });
});
