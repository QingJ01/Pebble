import { useState, useEffect } from "react";
import { useTranslation } from "react-i18next";
import {
  getTranslateConfig,
  saveTranslateConfig,
  testTranslateConnection,
} from "../../lib/api";

type ProviderType = "deeplx" | "deepl" | "generic_api" | "llm";

const PROVIDER_OPTIONS: { value: ProviderType; label: string }[] = [
  { value: "deeplx", label: "DeepLX" },
  { value: "deepl", label: "DeepL" },
  { value: "generic_api", label: "Generic API" },
  { value: "llm", label: "LLM" },
];

const labelStyle: React.CSSProperties = {
  display: "block",
  fontSize: "12px",
  fontWeight: 500,
  color: "var(--color-text-secondary)",
  marginBottom: "4px",
};

const inputStyle: React.CSSProperties = {
  width: "100%",
  padding: "8px 10px",
  fontSize: "13px",
  border: "1px solid var(--color-border)",
  borderRadius: "6px",
  background: "var(--color-bg-secondary)",
  color: "var(--color-text-primary)",
  boxSizing: "border-box",
};

const fieldGroupStyle: React.CSSProperties = {
  marginBottom: "14px",
};

const buttonStyle: React.CSSProperties = {
  padding: "8px 18px",
  fontSize: "13px",
  fontWeight: 500,
  border: "none",
  borderRadius: "6px",
  cursor: "pointer",
};

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
      const errMsg = err instanceof Error ? err.message : String(err);
      setStatusMsg(t("translate.saveFailed", { error: errMsg }));
      setStatusType("error");
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
      const errMsg = err instanceof Error ? err.message : String(err);
      setStatusMsg(t("translate.testFailed", { error: errMsg }));
      setStatusType("error");
    } finally {
      setTesting(false);
    }
  }

  function renderProviderFields() {
    switch (providerType) {
      case "deeplx":
        return (
          <div style={fieldGroupStyle}>
            <label htmlFor="translate-deeplx-endpoint" style={labelStyle}>{t("translate.endpointUrl")}</label>
            <input
              id="translate-deeplx-endpoint"
              name="deeplx_endpoint"
              type="url"
              style={inputStyle}
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
            <div style={fieldGroupStyle}>
              <label htmlFor="translate-deepl-api-key" style={labelStyle}>{t("translate.apiKey")}</label>
              <input
                id="translate-deepl-api-key"
                name="deepl_api_key"
                style={inputStyle}
                type="password"
                value={deeplApiKey}
                onChange={(e) => setDeeplApiKey(e.target.value)}
                placeholder="your-deepl-api-key"
                autoComplete="current-password"
              />
            </div>
            <div style={{ ...fieldGroupStyle, display: "flex", alignItems: "center", gap: "8px" }}>
              <input
                type="checkbox"
                checked={deeplUseFree}
                onChange={(e) => setDeeplUseFree(e.target.checked)}
                id="deepl-free"
              />
              <label htmlFor="deepl-free" style={{ fontSize: "13px", color: "var(--color-text-primary)", cursor: "pointer" }}>
                {t("translate.useFreeApi")}
              </label>
            </div>
          </>
        );

      case "generic_api":
        return (
          <>
            <div style={fieldGroupStyle}>
              <label htmlFor="translate-generic-endpoint" style={labelStyle}>{t("translate.endpoint")}</label>
              <input
                id="translate-generic-endpoint"
                name="generic_endpoint"
                type="url"
                style={inputStyle}
                value={genericEndpoint}
                onChange={(e) => setGenericEndpoint(e.target.value)}
                placeholder="https://api.example.com/translate"
                autoComplete="off"
              />
            </div>
            <div style={fieldGroupStyle}>
              <label htmlFor="translate-generic-api-key" style={labelStyle}>{t("translate.apiKeyOptional")}</label>
              <input
                id="translate-generic-api-key"
                name="generic_api_key"
                style={inputStyle}
                type="password"
                value={genericApiKey}
                onChange={(e) => setGenericApiKey(e.target.value)}
                placeholder={t("translate.apiKeyOptional")}
                autoComplete="current-password"
              />
            </div>
            <div style={fieldGroupStyle}>
              <label htmlFor="translate-generic-source-param" style={labelStyle}>{t("translate.sourceLangParam")}</label>
              <input
                id="translate-generic-source-param"
                name="generic_source_lang_param"
                style={inputStyle}
                value={genericSourceLangParam}
                onChange={(e) => setGenericSourceLangParam(e.target.value)}
                autoComplete="off"
              />
            </div>
            <div style={fieldGroupStyle}>
              <label htmlFor="translate-generic-target-param" style={labelStyle}>{t("translate.targetLangParam")}</label>
              <input
                id="translate-generic-target-param"
                name="generic_target_lang_param"
                style={inputStyle}
                value={genericTargetLangParam}
                onChange={(e) => setGenericTargetLangParam(e.target.value)}
                autoComplete="off"
              />
            </div>
            <div style={fieldGroupStyle}>
              <label htmlFor="translate-generic-text-param" style={labelStyle}>{t("translate.textParam")}</label>
              <input
                id="translate-generic-text-param"
                name="generic_text_param"
                style={inputStyle}
                value={genericTextParam}
                onChange={(e) => setGenericTextParam(e.target.value)}
                autoComplete="off"
              />
            </div>
            <div style={fieldGroupStyle}>
              <label htmlFor="translate-generic-result-path" style={labelStyle}>{t("translate.resultPath")}</label>
              <input
                id="translate-generic-result-path"
                name="generic_result_path"
                style={inputStyle}
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
            <div style={fieldGroupStyle}>
              <label htmlFor="translate-llm-endpoint" style={labelStyle}>{t("translate.endpoint")}</label>
              <input
                id="translate-llm-endpoint"
                name="llm_endpoint"
                type="url"
                style={inputStyle}
                value={llmEndpoint}
                onChange={(e) => setLlmEndpoint(e.target.value)}
                placeholder="https://api.openai.com/v1"
                autoComplete="off"
              />
            </div>
            <div style={fieldGroupStyle}>
              <label htmlFor="translate-llm-api-key" style={labelStyle}>{t("translate.apiKey")}</label>
              <input
                id="translate-llm-api-key"
                name="llm_api_key"
                style={inputStyle}
                type="password"
                value={llmApiKey}
                onChange={(e) => setLlmApiKey(e.target.value)}
                placeholder="your-api-key"
                autoComplete="current-password"
              />
            </div>
            <div style={fieldGroupStyle}>
              <label htmlFor="translate-llm-model" style={labelStyle}>{t("translate.model")}</label>
              <input
                id="translate-llm-model"
                name="llm_model"
                style={inputStyle}
                value={llmModel}
                onChange={(e) => setLlmModel(e.target.value)}
                placeholder="gpt-4o-mini"
                autoComplete="off"
              />
            </div>
            <div style={fieldGroupStyle}>
              <label htmlFor="translate-llm-mode" style={labelStyle}>{t("translate.mode")}</label>
              <select
                id="translate-llm-mode"
                name="llm_mode"
                style={inputStyle}
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
      <h2 style={{ fontSize: "18px", fontWeight: 600, color: "var(--color-text-primary)", marginTop: 0, marginBottom: "20px" }}>
        {t("translate.engineTitle")}
      </h2>

      {/* Enable toggle */}
      <div style={{ ...fieldGroupStyle, display: "flex", alignItems: "center", gap: "8px" }}>
        <input
          type="checkbox"
          checked={isEnabled}
          onChange={(e) => setIsEnabled(e.target.checked)}
          id="translate-enabled"
        />
        <label htmlFor="translate-enabled" style={{ fontSize: "13px", color: "var(--color-text-primary)", cursor: "pointer" }}>
          {t("translate.enableTranslation")}
        </label>
      </div>

      {/* Provider selector */}
      <div style={fieldGroupStyle}>
        <label htmlFor="translate-provider" style={labelStyle}>{t("translate.provider")}</label>
        <select
          id="translate-provider"
          name="translate_provider"
          style={inputStyle}
          value={providerType}
          onChange={(e) => setProviderType(e.target.value as ProviderType)}
        >
          {PROVIDER_OPTIONS.map((opt) => (
            <option key={opt.value} value={opt.value}>
              {opt.label}
            </option>
          ))}
        </select>
      </div>

      {/* Dynamic provider fields */}
      {renderProviderFields()}

      {/* Actions */}
      <div style={{ display: "flex", gap: "10px", marginTop: "20px" }}>
        <button
          style={{
            ...buttonStyle,
            background: "var(--color-accent)",
            color: "#fff",
            opacity: saving ? 0.6 : 1,
          }}
          onClick={handleSave}
          disabled={saving}
        >
          {saving ? t("common.saving") : t("common.save")}
        </button>
        <button
          style={{
            ...buttonStyle,
            background: "var(--color-bg-hover)",
            color: "var(--color-text-primary)",
            opacity: testing ? 0.6 : 1,
          }}
          onClick={handleTest}
          disabled={testing}
        >
          {testing ? t("common.testing") : t("translate.testConnection")}
        </button>
      </div>

      {/* Status message */}
      {statusMsg && (
        <div
          role={statusType === "error" ? "alert" : "status"}
          aria-live="polite"
          style={{
            marginTop: "14px",
            padding: "10px 14px",
            borderRadius: "6px",
            fontSize: "13px",
            background: statusType === "success" ? "var(--color-bg-hover)" : "rgba(220, 53, 69, 0.1)",
            color: statusType === "success" ? "var(--color-text-primary)" : "#dc3545",
            border: `1px solid ${statusType === "success" ? "var(--color-border)" : "rgba(220, 53, 69, 0.3)"}`,
          }}
        >
          {statusMsg}
        </div>
      )}
    </div>
  );
}
