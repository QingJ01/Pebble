import { useState, useEffect } from "react";
import { useTranslation } from "react-i18next";
import {
  getTranslateConfig,
  saveTranslateConfig,
  testTranslateConnection,
} from "../../lib/api";
import { useToastStore } from "@/stores/toast.store";
import { extractErrorMessage } from "../../lib/extractErrorMessage";
import Button from "@/components/ui/Button";
import Checkbox from "@/components/ui/Checkbox";
import { SettingsNotice, SettingsSection } from "./SettingsLayout";

type ProviderType = "deeplx" | "deepl" | "generic_api" | "llm";

const PROVIDER_OPTIONS: { value: ProviderType; label: string; labelKey?: string }[] = [
  { value: "deeplx", label: "DeepLX" },
  { value: "deepl", label: "DeepL" },
  { value: "generic_api", label: "Generic API", labelKey: "translate.providerGenericApi" },
  { value: "llm", label: "LLM", labelKey: "translate.providerLlm" },
];

export default function TranslateTab() {
  const { t } = useTranslation();
  const [providerType, setProviderType] = useState<ProviderType>("deeplx");
  const [isEnabled, setIsEnabled] = useState(false);

  // DeepLX fields
  const [deeplxEndpoint, setDeeplxEndpoint] = useState("");

  // DeepL fields
  const [deeplApiKey, setDeeplApiKey] = useState("");
  const [deeplUseFree, setDeeplUseFree] = useState(false);

  // Generic API fields
  const [genericEndpoint, setGenericEndpoint] = useState("");
  const [genericApiKey, setGenericApiKey] = useState("");
  const [genericSourceLangParam, setGenericSourceLangParam] = useState("source_lang");
  const [genericTargetLangParam, setGenericTargetLangParam] = useState("target_lang");
  const [genericTextParam, setGenericTextParam] = useState("text");
  const [genericResultPath, setGenericResultPath] = useState("data");

  // LLM fields
  const [llmEndpoint, setLlmEndpoint] = useState("");
  const [llmApiKey, setLlmApiKey] = useState("");
  const [llmModel, setLlmModel] = useState("");
  const [llmMode, setLlmMode] = useState<"completions" | "responses">("completions");

  // The backend never returns the saved API key, only `has_api_key`. A blank
  // key field then keeps the saved key (same provider and endpoint only).
  const [savedKeyProvider, setSavedKeyProvider] = useState<ProviderType | null>(null);

  const [statusMsg, setStatusMsg] = useState("");
  const [statusType, setStatusType] = useState<"success" | "error" | "">("");
  const [saving, setSaving] = useState(false);
  const [testing, setTesting] = useState(false);

  useEffect(() => {
    loadConfig();
  }, []);

  async function loadConfig() {
    try {
      const cfg = await getTranslateConfig();
      if (!cfg) return;

      setProviderType(cfg.provider_type as ProviderType);
      setIsEnabled(cfg.is_enabled);

      const parsed = JSON.parse(cfg.config);
      setSavedKeyProvider(parsed.has_api_key ? (parsed.type as ProviderType) : null);
      switch (parsed.type) {
        case "deeplx":
          setDeeplxEndpoint(parsed.endpoint || "");
          break;
        case "deepl":
          setDeeplApiKey(parsed.api_key || "");
          setDeeplUseFree(parsed.use_free_api ?? false);
          break;
        case "generic_api":
          setGenericEndpoint(parsed.endpoint || "");
          setGenericApiKey(parsed.api_key || "");
          setGenericSourceLangParam(parsed.source_lang_param || "source_lang");
          setGenericTargetLangParam(parsed.target_lang_param || "target_lang");
          setGenericTextParam(parsed.text_param || "text");
          setGenericResultPath(parsed.result_path || "data");
          break;
        case "llm":
          setLlmEndpoint(parsed.endpoint || "");
          setLlmApiKey(parsed.api_key || "");
          setLlmModel(parsed.model || "");
          setLlmMode(parsed.mode || "completions");
          break;
      }
    } catch (err) {
      console.error("Failed to load translate config:", err);
    }
  }

  function buildConfigJson(): string {
    switch (providerType) {
      case "deeplx":
        return JSON.stringify({ type: "deeplx", endpoint: deeplxEndpoint });
      case "deepl":
        return JSON.stringify({ type: "deepl", api_key: deeplApiKey, use_free_api: deeplUseFree });
      case "generic_api":
        return JSON.stringify({
          type: "generic_api",
          endpoint: genericEndpoint,
          api_key: genericApiKey || null,
          method: null,
          source_lang_param: genericSourceLangParam,
          target_lang_param: genericTargetLangParam,
          text_param: genericTextParam,
          result_path: genericResultPath,
        });
      case "llm":
        return JSON.stringify({
          type: "llm",
          endpoint: llmEndpoint,
          api_key: llmApiKey,
          model: llmModel,
          mode: llmMode,
        });
    }
  }

  async function handleSave() {
    setSaving(true);
    setStatusMsg("");
    try {
      const configJson = buildConfigJson();
      await saveTranslateConfig(providerType, configJson, isEnabled);
      setStatusMsg(t("translate.configSaved"));
      setStatusType("success");
    } catch (err: unknown) {
      const errMsg = extractErrorMessage(err);
      setStatusMsg(t("translate.saveFailed", { error: errMsg }));
      setStatusType("error");
      useToastStore.getState().addToast({
        message: t("translate.saveFailed", { error: errMsg }),
        type: "error",
      });
    } finally {
      setSaving(false);
    }
  }

  async function handleTest() {
    setTesting(true);
    setStatusMsg("");
    try {
      const configJson = buildConfigJson();
      const result = await testTranslateConnection(configJson);
      setStatusMsg(t("translate.connectionOk", { result }));
      setStatusType("success");
    } catch (err: unknown) {
      const errMsg = extractErrorMessage(err);
      setStatusMsg(t("translate.testFailed", { error: errMsg }));
      setStatusType("error");
      useToastStore.getState().addToast({
        message: t("translate.testFailed", { error: errMsg }),
        type: "error",
      });
    } finally {
      setTesting(false);
    }
  }

  function apiKeyPlaceholder(provider: ProviderType, fallback: string) {
    return savedKeyProvider === provider
      ? t("translate.savedApiKeyPlaceholder", "Saved — leave blank to keep")
      : fallback;
  }

  function renderProviderFields() {
    switch (providerType) {
      case "deeplx":
        return (
          <div className="field">
            <label htmlFor="translate-deeplx-endpoint" className="field__label">{t("translate.endpointUrl")}</label>
            <input
              id="translate-deeplx-endpoint"
              name="deeplx_endpoint"
              type="url"
              className="input"
              value={deeplxEndpoint}
              onChange={(e) => setDeeplxEndpoint(e.target.value)}
              placeholder="http://localhost:1188/translate"
              autoComplete="off"
            />
          </div>
        );

      case "deepl":
        return (
          <>
            <div className="field">
              <label htmlFor="translate-deepl-api-key" className="field__label">{t("translate.apiKey")}</label>
              <input
                id="translate-deepl-api-key"
                name="deepl_api_key"
                className="input"
                type="password"
                value={deeplApiKey}
                onChange={(e) => setDeeplApiKey(e.target.value)}
                placeholder={apiKeyPlaceholder("deepl", "your-deepl-api-key")}
                autoComplete="current-password"
              />
            </div>
            <Checkbox
              checked={deeplUseFree}
              onChange={(e) => setDeeplUseFree(e.target.checked)}
              id="deepl-free"
              label={t("translate.useFreeApi")}
            />
          </>
        );

      case "generic_api":
        return (
          <>
            <div className="field">
              <label htmlFor="translate-generic-endpoint" className="field__label">{t("translate.endpoint")}</label>
              <input
                id="translate-generic-endpoint"
                name="generic_endpoint"
                type="url"
                className="input"
                value={genericEndpoint}
                onChange={(e) => setGenericEndpoint(e.target.value)}
                placeholder="https://api.example.com/translate"
                autoComplete="off"
              />
            </div>
            <div className="field">
              <label htmlFor="translate-generic-api-key" className="field__label">{t("translate.apiKeyOptional")}</label>
              <input
                id="translate-generic-api-key"
                name="generic_api_key"
                className="input"
                type="password"
                value={genericApiKey}
                onChange={(e) => setGenericApiKey(e.target.value)}
                placeholder={apiKeyPlaceholder("generic_api", t("translate.apiKeyOptional"))}
                autoComplete="current-password"
              />
            </div>
            <div className="field">
              <label htmlFor="translate-generic-source-param" className="field__label">{t("translate.sourceLangParam")}</label>
              <input
                id="translate-generic-source-param"
                name="generic_source_lang_param"
                className="input"
                value={genericSourceLangParam}
                onChange={(e) => setGenericSourceLangParam(e.target.value)}
                autoComplete="off"
              />
            </div>
            <div className="field">
              <label htmlFor="translate-generic-target-param" className="field__label">{t("translate.targetLangParam")}</label>
              <input
                id="translate-generic-target-param"
                name="generic_target_lang_param"
                className="input"
                value={genericTargetLangParam}
                onChange={(e) => setGenericTargetLangParam(e.target.value)}
                autoComplete="off"
              />
            </div>
            <div className="field">
              <label htmlFor="translate-generic-text-param" className="field__label">{t("translate.textParam")}</label>
              <input
                id="translate-generic-text-param"
                name="generic_text_param"
                className="input"
                value={genericTextParam}
                onChange={(e) => setGenericTextParam(e.target.value)}
                autoComplete="off"
              />
            </div>
            <div className="field">
              <label htmlFor="translate-generic-result-path" className="field__label">{t("translate.resultPath")}</label>
              <input
                id="translate-generic-result-path"
                name="generic_result_path"
                className="input"
                value={genericResultPath}
                onChange={(e) => setGenericResultPath(e.target.value)}
                autoComplete="off"
              />
            </div>
          </>
        );

      case "llm":
        return (
          <>
            <div className="field">
              <label htmlFor="translate-llm-endpoint" className="field__label">{t("translate.endpoint")}</label>
              <input
                id="translate-llm-endpoint"
                name="llm_endpoint"
                type="url"
                className="input"
                value={llmEndpoint}
                onChange={(e) => setLlmEndpoint(e.target.value)}
                placeholder="https://api.openai.com/v1"
                autoComplete="off"
              />
            </div>
            <div className="field">
              <label htmlFor="translate-llm-api-key" className="field__label">{t("translate.apiKey")}</label>
              <input
                id="translate-llm-api-key"
                name="llm_api_key"
                className="input"
                type="password"
                value={llmApiKey}
                onChange={(e) => setLlmApiKey(e.target.value)}
                placeholder={apiKeyPlaceholder("llm", "your-api-key")}
                autoComplete="current-password"
              />
            </div>
            <div className="field">
              <label htmlFor="translate-llm-model" className="field__label">{t("translate.model")}</label>
              <input
                id="translate-llm-model"
                name="llm_model"
                className="input"
                value={llmModel}
                onChange={(e) => setLlmModel(e.target.value)}
                placeholder="gpt-4o-mini"
                autoComplete="off"
              />
            </div>
            <div className="field">
              <label htmlFor="translate-llm-mode" className="field__label">{t("translate.mode")}</label>
              <select
                id="translate-llm-mode"
                name="llm_mode"
                className="select"
                value={llmMode}
                onChange={(e) => setLlmMode(e.target.value as "completions" | "responses")}
              >
                <option value="completions">{t("translate.modeCompletions")}</option>
                <option value="responses">{t("translate.modeResponses")}</option>
              </select>
            </div>
          </>
        );
    }
  }

  return (
    <div>
      <SettingsSection title={t("translate.engineTitle")}>
        <Checkbox
          checked={isEnabled}
          onChange={(e) => setIsEnabled(e.target.checked)}
          id="translate-enabled"
          label={t("translate.enableTranslation")}
        />

        <div className="field">
          <label htmlFor="translate-provider" className="field__label">{t("translate.provider")}</label>
          <select
            id="translate-provider"
            name="translate_provider"
            className="select"
            value={providerType}
            onChange={(e) => setProviderType(e.target.value as ProviderType)}
          >
            {PROVIDER_OPTIONS.map((opt) => (
              <option key={opt.value} value={opt.value}>
                {opt.labelKey ? t(opt.labelKey, opt.label) : opt.label}
              </option>
            ))}
          </select>
        </div>

        {renderProviderFields()}

        <div className="settings-actions" style={{ marginTop: "var(--space-2)" }}>
          <Button variant="primary" onClick={handleSave} loading={saving}>
            {saving ? t("common.saving") : t("common.save")}
          </Button>
          <Button onClick={handleTest} loading={testing}>
            {testing ? t("common.testing") : t("translate.testConnection")}
          </Button>
        </div>

        {statusMsg && (
          <SettingsNotice
            tone={statusType === "error" ? "danger" : "success"}
            role={statusType === "error" ? "alert" : "status"}
            aria-live="polite"
          >
            {statusMsg}
          </SettingsNotice>
        )}
      </SettingsSection>
    </div>
  );
}
