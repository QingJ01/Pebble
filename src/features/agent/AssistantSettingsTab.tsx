import { useEffect, useState, type FormEvent } from "react";
import { useTranslation } from "react-i18next";
import { CheckCircle2, XCircle } from "lucide-react";
import Button from "@/components/ui/Button";
import Input from "@/components/ui/Input";
import { useToastStore } from "@/stores/toast.store";
import { SettingsNotice, SettingsSection } from "@/features/settings/SettingsLayout";
import { useAgentModel, useAgentModelMutations, type AgentModelTest } from "./model-api";
import "./agent.css";

const PRESETS: Array<{ key: string; label: string; endpoint: string; model: string }> = [
  { key: "ollama", label: "Ollama", endpoint: "http://localhost:11434", model: "qwen2.5:7b" },
  { key: "lmstudio", label: "LM Studio", endpoint: "http://localhost:1234", model: "" },
  { key: "openai", label: "OpenAI", endpoint: "https://api.openai.com", model: "gpt-4o-mini" },
  { key: "openrouter", label: "OpenRouter", endpoint: "https://openrouter.ai/api/v1", model: "" },
  { key: "deepseek", label: "DeepSeek", endpoint: "https://api.deepseek.com", model: "deepseek-chat" },
];

/** Settings → AI assistant: the model the assistant talks to. */
export default function AssistantSettingsTab() {
  const { t } = useTranslation();
  const addToast = useToastStore((s) => s.addToast);
  const { data: saved } = useAgentModel();
  const { save, clear, importFromTranslate, test } = useAgentModelMutations();
  const [endpoint, setEndpoint] = useState("");
  const [model, setModel] = useState("");
  const [apiKey, setApiKey] = useState("");
  const [result, setResult] = useState<AgentModelTest | null>(null);

  useEffect(() => {
    if (!saved) return;
    setEndpoint(saved.endpoint);
    setModel(saved.model);
  }, [saved]);

  const dirty = !!saved && (endpoint.trim() !== saved.endpoint || model.trim() !== saved.model || apiKey.trim() !== "");
  const canSave = endpoint.trim() !== "" && model.trim() !== "";

  function handleSave(event: FormEvent) {
    event.preventDefault();
    save.mutate(
      { endpoint, model, apiKey: apiKey || undefined },
      {
        onSuccess: () => {
          setApiKey("");
          addToast({ message: t("assistantSettings.saved", "Assistant model saved"), type: "success" });
        },
        onError: (error) => addToast({ message: String(error), type: "error" }),
      },
    );
  }

  function handleTest() {
    setResult(null);
    test.mutate(
      { endpoint: endpoint || undefined, model: model || undefined, apiKey: apiKey || undefined },
      {
        onSuccess: setResult,
        onError: (error) => setResult({ ok: false, latencyMs: 0, message: String(error) }),
      },
    );
  }

  return (
    <div>
      <SettingsSection
        title={t("assistantSettings.modelTitle", "Model")}
        description={t(
          "assistantSettings.modelDesc",
          "Any OpenAI-compatible service works, in the cloud or on this computer (Ollama, LM Studio). Pebble's reminders and briefings work without a model; it's needed to ask Pebble questions.",
        )}
      >
        <div className="assistant-presets" role="group" aria-label={t("assistantSettings.presets", "Presets")}>
          {PRESETS.map((preset) => (
            <button
              key={preset.key}
              type="button"
              className="agent-chip"
              onClick={() => {
                setEndpoint(preset.endpoint);
                if (preset.model) setModel(preset.model);
                setResult(null);
              }}
            >
              {preset.label}
            </button>
          ))}
          <button
            type="button"
            className="agent-chip"
            onClick={() =>
              importFromTranslate.mutate(undefined, {
                onSuccess: (view) =>
                  addToast(
                    view
                      ? { message: t("assistantSettings.imported", "Copied the model from Translation settings"), type: "success" }
                      : { message: t("assistantSettings.nothingToImport", "Translation doesn't use an AI model"), type: "info" },
                  ),
              })
            }
          >
            {t("assistantSettings.importTranslate", "Use the translation model")}
          </button>
        </div>
        <form className="assistant-form" onSubmit={handleSave}>
          <label className="field">
            <span className="field__label">{t("assistantSettings.endpoint", "Endpoint")}</span>
            <Input value={endpoint} onChange={(e) => setEndpoint(e.target.value)} placeholder="https://api.openai.com" spellCheck={false} />
          </label>
          <label className="field">
            <span className="field__label">{t("assistantSettings.model", "Model name")}</span>
            <Input value={model} onChange={(e) => setModel(e.target.value)} placeholder="gpt-4o-mini" spellCheck={false} />
          </label>
          <label className="field">
            <span className="field__label">{t("assistantSettings.apiKey", "API key")}</span>
            <Input
              type="password"
              value={apiKey}
              onChange={(e) => setApiKey(e.target.value)}
              placeholder={saved?.hasApiKey
                ? t("assistantSettings.apiKeySaved", "Saved — leave empty to keep it")
                : t("assistantSettings.apiKeyOptional", "Not needed for local models")}
              autoComplete="off"
            />
          </label>
          <div className="settings-actions">
            <Button type="submit" variant="primary" disabled={!canSave || (!!saved?.configured && !dirty)} loading={save.isPending}>
              {t("assistantSettings.save", "Save")}
            </Button>
            <Button type="button" onClick={handleTest} disabled={!canSave} loading={test.isPending}>
              {t("assistantSettings.test", "Test connection")}
            </Button>
            {saved?.configured && (
              <Button type="button" variant="danger-ghost" onClick={() => clear.mutate()}>
                {t("assistantSettings.remove", "Remove")}
              </Button>
            )}
          </div>
          {result && (
            <p className={`assistant-test${result.ok ? " is-ok" : " is-error"}`} role="status">
              {result.ok ? <CheckCircle2 size={14} aria-hidden="true" /> : <XCircle size={14} aria-hidden="true" />}
              {result.ok
                ? t("assistantSettings.testOk", "Connected · answered in {{ms}} ms", { ms: result.latencyMs })
                : t("assistantSettings.testFailed", "Couldn't reach the model: {{message}}", { message: result.message })}
            </p>
          )}
        </form>
      </SettingsSection>

      <SettingsSection title={t("assistantSettings.privacyTitle", "What the model sees")}>
        <SettingsNotice tone={saved?.local ? "success" : "neutral"}>
          {saved?.configured && saved.local
            ? t("assistantSettings.privacyLocal", "This model runs on your computer: your mail stays on this device.")
            : t(
              "assistantSettings.privacyCloud",
              "When you ask Pebble something, the mail it needs to answer is sent to this service. Nothing is sent until you ask, and each answer shows which messages were used.",
            )}
        </SettingsNotice>
      </SettingsSection>
    </div>
  );
}
