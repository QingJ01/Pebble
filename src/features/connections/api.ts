import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useEffect } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

/** Access a paired MCP client has to one account, lowest first. */
export type McpLevel = "metadata" | "summary" | "body" | "drafts";
export const MCP_LEVELS: McpLevel[] = ["metadata", "summary", "body", "drafts"];

export interface McpStatus {
  enabled: boolean;
  running: boolean;
  endpoint: string;
  command: string;
  args: string[];
  configJson: string;
}

export interface McpClient {
  id: string;
  name: string;
  /** Account id → level. */
  grants: Record<string, McpLevel>;
  createdAt: number;
  lastUsedAt: number | null;
  revokedAt: number | null;
}

export interface McpAuditRow {
  id: number;
  clientId: string;
  tool: string;
  accounts: string;
  results: number;
  outcome: string;
  createdAt: number;
}

export interface PendingPair {
  id: string;
  client: string;
  createdAt: number;
}

export const MCP_PAIR_EVENT = "mcp:pair-request";
export const MCP_CHANGED_EVENT = "mcp:changed";

export const mcpKeys = {
  all: ["mcp"] as const,
  status: ["mcp", "status"] as const,
  clients: ["mcp", "clients"] as const,
  audit: ["mcp", "audit"] as const,
  pending: ["mcp", "pending"] as const,
};

export const getMcpStatus = () => invoke<McpStatus>("mcp_get_status");
export const setMcpEnabled = (enabled: boolean) => invoke<void>("mcp_set_enabled", { enabled });
export const listPendingPairs = () => invoke<PendingPair[]>("mcp_pending_pairs");
export const respondToPair = (id: string, approve: boolean, grants: Record<string, McpLevel>) =>
  invoke<void>("mcp_respond_pair", { id, approve, grants });
export const listMcpClients = () => invoke<McpClient[]>("mcp_list_clients");
export const setMcpGrants = (clientId: string, grants: Record<string, McpLevel>) =>
  invoke<void>("mcp_set_grants", { clientId, grants });
export const revokeMcpClient = (clientId: string) => invoke<void>("mcp_revoke", { clientId });
export const listMcpAudit = (limit = 50) => invoke<McpAuditRow[]>("mcp_list_audit", { clientId: null, limit });

/** Refetch MCP data when the backend reports a change or a pairing request. */
export function useMcpInvalidation() {
  const queryClient = useQueryClient();
  useEffect(() => {
    let disposed = false;
    const unlisteners: Array<() => void> = [];
    for (const event of [MCP_CHANGED_EVENT, MCP_PAIR_EVENT]) {
      void listen(event, () => {
        void queryClient.invalidateQueries({ queryKey: mcpKeys.all });
      }).then((fn) => {
        if (disposed) fn();
        else unlisteners.push(fn);
      }).catch(() => {});
    }
    return () => {
      disposed = true;
      unlisteners.forEach((fn) => fn());
    };
  }, [queryClient]);
}

export const useMcpStatus = () => useQuery({ queryKey: mcpKeys.status, queryFn: getMcpStatus });
export const useMcpClients = () => useQuery({ queryKey: mcpKeys.clients, queryFn: listMcpClients });
export const useMcpAudit = () => useQuery({ queryKey: mcpKeys.audit, queryFn: () => listMcpAudit(50) });
export const usePendingPairs = () => useQuery({ queryKey: mcpKeys.pending, queryFn: listPendingPairs });

export function useMcpMutations() {
  const queryClient = useQueryClient();
  const refresh = () => queryClient.invalidateQueries({ queryKey: mcpKeys.all });
  return {
    setEnabled: useMutation({ mutationFn: setMcpEnabled, onSettled: refresh }),
    respond: useMutation({
      mutationFn: ({ id, approve, grants }: { id: string; approve: boolean; grants: Record<string, McpLevel> }) =>
        respondToPair(id, approve, grants),
      onSettled: refresh,
    }),
    setGrants: useMutation({
      mutationFn: ({ clientId, grants }: { clientId: string; grants: Record<string, McpLevel> }) => setMcpGrants(clientId, grants),
      onSettled: refresh,
    }),
    revoke: useMutation({ mutationFn: revokeMcpClient, onSettled: refresh }),
  };
}
