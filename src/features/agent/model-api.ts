import { invoke } from "@tauri-apps/api/core";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

/** Mirrors `commands::agent_model::AgentModelView` (no API key, ever). */
export interface AgentModel {
  configured: boolean;
  endpoint: string;
  model: string;
  hasApiKey: boolean;
  local: boolean;
}

export interface AgentModelTest {
  ok: boolean;
  latencyMs: number;
  message: string;
}

export const agentModelKey = ["agent", "model"] as const;

export const getAgentModel = () => invoke<AgentModel>("get_agent_model");
export const setAgentModel = (input: { endpoint: string; model: string; apiKey?: string; clearApiKey?: boolean }) =>
  invoke<AgentModel>("set_agent_model", {
    endpoint: input.endpoint,
    model: input.model,
    apiKey: input.apiKey ?? null,
    clearApiKey: input.clearApiKey ?? null,
  });
export const clearAgentModel = () => invoke<void>("clear_agent_model");
export const importAgentModelFromTranslate = () => invoke<AgentModel | null>("import_agent_model_from_translate");
export const testAgentModel = (input: { endpoint?: string; model?: string; apiKey?: string }) =>
  invoke<AgentModelTest>("test_agent_model", {
    endpoint: input.endpoint ?? null,
    model: input.model ?? null,
    apiKey: input.apiKey ?? null,
  });

export function useAgentModel() {
  return useQuery({ queryKey: agentModelKey, queryFn: getAgentModel });
}

export function useAgentModelMutations() {
  const queryClient = useQueryClient();
  const refresh = () => queryClient.invalidateQueries({ queryKey: agentModelKey });
  return {
    save: useMutation({ mutationFn: setAgentModel, onSettled: refresh }),
    clear: useMutation({ mutationFn: clearAgentModel, onSettled: refresh }),
    importFromTranslate: useMutation({ mutationFn: importAgentModelFromTranslate, onSettled: refresh }),
    test: useMutation({ mutationFn: testAgentModel }),
  };
}
