import { useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Image, Trash2, Upload } from "lucide-react";
import { useUIStore } from "@/stores/ui.store";
import type { BackgroundImageFit, Language, SidebarStyle, Theme } from "@/stores/ui.store";
import { backgroundImageUrl, deleteBackgroundImage, importBackgroundImage } from "@/lib/backgroundImage";

const THEMES: { id: Theme; labelKey: string; descKey: string }[] = [
  { id: "light", labelKey: "settings.themeLight", descKey: "settings.themeLightDesc" },
  { id: "dark", labelKey: "settings.themeDark", descKey: "settings.themeDarkDesc" },
  { id: "system", labelKey: "settings.themeSystem", descKey: "settings.themeSystemDesc" },
];

const LANGUAGES: { id: Language; label: string }[] = [
  { id: "en", label: "English" },
  { id: "zh", label: "\u4e2d\u6587" },
];

const BACKGROUND_FITS: { id: BackgroundImageFit; labelKey: string; fallback: string }[] = [
  { id: "cover", labelKey: "settings.backgroundFitCover", fallback: "Fill" },
  { id: "contain", labelKey: "settings.backgroundFitContain", fallback: "Fit" },
  { id: "repeat", labelKey: "settings.backgroundFitRepeat", fallback: "Tile" },
];

const SIDEBAR_STYLES: { id: SidebarStyle; labelKey: string; descKey: string }[] = [
  { id: "grouped", labelKey: "settings.sidebarStyleGrouped", descKey: "settings.sidebarStyleGroupedDesc" },
  { id: "classic", labelKey: "settings.sidebarStyleClassic", descKey: "settings.sidebarStyleClassicDesc" },
];

function errorMessage(error: unknown): string {
  if (error instanceof Error) return error.message;
  if (typeof error === "string") return error;
  return "Failed to update background image";
}

export default function AppearanceTab() {
  const { t } = useTranslation();
  const fileInputRef = useRef<HTMLInputElement | null>(null);
  const [uploading, setUploading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const theme = useUIStore((s) => s.theme);
  const setTheme = useUIStore((s) => s.setTheme);
  const language = useUIStore((s) => s.language);
  const setLanguage = useUIStore((s) => s.setLanguage);
  const backgroundImage = useUIStore((s) => s.backgroundImage);
  const sidebarStyle = useUIStore((s) => s.sidebarStyle);
  const setSidebarStyle = useUIStore((s) => s.setSidebarStyle);
  const setBackgroundImage = useUIStore((s) => s.setBackgroundImage);
  const setBackgroundImageFit = useUIStore((s) => s.setBackgroundImageFit);
  const setBackgroundImageOpacity = useUIStore((s) => s.setBackgroundImageOpacity);
  const clearBackgroundImage = useUIStore((s) => s.clearBackgroundImage);

  async function handleBackgroundFileChange(event: React.ChangeEvent<HTMLInputElement>) {
    const file = event.currentTarget.files?.[0];
    event.currentTarget.value = "";
    if (!file) return;

    setUploading(true);
    setError(null);
    try {
      const imported = await importBackgroundImage(file);
      setBackgroundImage({
        path: imported.path,
        filename: imported.filename,
      });
    } catch (err) {
      setError(errorMessage(err));
    } finally {
      setUploading(false);
    }
  }

  async function handleRemoveBackgroundImage() {
    const existing = backgroundImage;
    if (!existing) return;

    setUploading(true);
    setError(null);
    try {
      await deleteBackgroundImage(existing.path);
      clearBackgroundImage();
    } catch (err) {
      setError(errorMessage(err));
    } finally {
      setUploading(false);
    }
  }

  return (
    <div>
      <h3 style={{ fontSize: "14px", fontWeight: 600, marginBottom: "16px" }}>{t("settings.theme")}</h3>
      <div style={{ display: "flex", gap: "12px" }}>
        {THEMES.map((th) => (
          <button
            key={th.id}
            onClick={() => setTheme(th.id)}
            style={{
              flex: 1,
              padding: "16px",
              borderRadius: "8px",
              border: theme === th.id ? "2px solid var(--color-accent)" : "1px solid var(--color-border)",
              backgroundColor: theme === th.id ? "var(--color-bg-hover)" : "transparent",
              cursor: "pointer",
              textAlign: "left",
              color: "var(--color-text-primary)",
            }}
          >
            <div style={{ fontWeight: 600, fontSize: "13px", marginBottom: "4px" }}>{t(th.labelKey)}</div>
            <div style={{ fontSize: "12px", color: "var(--color-text-secondary)" }}>{t(th.descKey)}</div>
          </button>
        ))}
      </div>

      <h3 style={{ fontSize: "14px", fontWeight: 600, marginBottom: "16px", marginTop: "32px" }}>
        {t("settings.language")}
      </h3>
      <div style={{ display: "flex", gap: "12px" }}>
        {LANGUAGES.map((l) => (
          <button
            key={l.id}
            onClick={() => setLanguage(l.id)}
            style={{
              flex: 1,
              padding: "16px",
              borderRadius: "8px",
              border: language === l.id ? "2px solid var(--color-accent)" : "1px solid var(--color-border)",
              backgroundColor: language === l.id ? "var(--color-bg-hover)" : "transparent",
              cursor: "pointer",
              textAlign: "left",
              color: "var(--color-text-primary)",
            }}
          >
            <div style={{ fontWeight: 600, fontSize: "13px" }}>{l.label}</div>
          </button>
        ))}
      </div>

      <h3 style={{ fontSize: "14px", fontWeight: 600, marginBottom: "16px", marginTop: "32px" }}>
        {t("settings.sidebarStyle", "Sidebar style")}
      </h3>
      <div
        role="group"
        aria-label={t("settings.sidebarStyle", "Sidebar style")}
        style={{ display: "flex", gap: "12px" }}
      >
        {SIDEBAR_STYLES.map((style) => (
          <button
            key={style.id}
            type="button"
            onClick={() => setSidebarStyle(style.id)}
            style={{
              flex: 1,
              padding: "16px",
              borderRadius: "8px",
              border: sidebarStyle === style.id ? "2px solid var(--color-accent)" : "1px solid var(--color-border)",
              backgroundColor: sidebarStyle === style.id ? "var(--color-bg-hover)" : "transparent",
              cursor: "pointer",
              textAlign: "left",
              color: "var(--color-text-primary)",
            }}
          >
            <div style={{ fontWeight: 600, fontSize: "13px", marginBottom: "4px" }}>{t(style.labelKey)}</div>
            <div style={{ fontSize: "12px", color: "var(--color-text-secondary)" }}>{t(style.descKey)}</div>
          </button>
        ))}
      </div>

      <h3 style={{ fontSize: "14px", fontWeight: 600, marginBottom: "16px", marginTop: "32px" }}>
        {t("settings.backgroundImage", "Background image")}
      </h3>
      <div
        style={{
          border: "1px solid var(--color-border)",
          borderRadius: "8px",
          padding: "14px",
          display: "grid",
          gap: "12px",
        }}
      >
        <div style={{ display: "flex", alignItems: "center", gap: "12px", minWidth: 0 }}>
          <div
            aria-hidden="true"
            style={{
              width: "64px",
              height: "42px",
              flexShrink: 0,
              borderRadius: "6px",
              border: "1px solid var(--color-border)",
              backgroundColor: "var(--color-bg-secondary)",
              backgroundImage: backgroundImage ? `url("${backgroundImageUrl(backgroundImage.path)}")` : undefined,
              backgroundPosition: "center",
              backgroundRepeat: backgroundImage?.fit === "repeat" ? "repeat" : "no-repeat",
              backgroundSize: backgroundImage?.fit === "repeat" ? "auto" : backgroundImage?.fit ?? "cover",
              display: "flex",
              alignItems: "center",
              justifyContent: "center",
              color: "var(--color-text-secondary)",
              overflow: "hidden",
            }}
          >
            {!backgroundImage && <Image size={18} />}
          </div>
          <div style={{ minWidth: 0, flex: 1 }}>
            <div style={{ fontSize: "13px", fontWeight: 600, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
              {backgroundImage?.filename ?? t("settings.noBackgroundImage", "No background image")}
            </div>
            <div style={{ fontSize: "12px", color: "var(--color-text-secondary)", marginTop: "3px" }}>
              {t("settings.backgroundImageDesc", "Stored locally in Pebble's app data folder.")}
            </div>
          </div>
        </div>

        <input
          ref={fileInputRef}
          aria-label={t("settings.chooseBackgroundImage", "Choose background image")}
          type="file"
          accept="image/png,image/jpeg,image/webp,image/gif"
          onChange={handleBackgroundFileChange}
          style={{ display: "none" }}
        />

        <div style={{ display: "flex", flexWrap: "wrap", gap: "8px" }}>
          <button
            type="button"
            onClick={() => fileInputRef.current?.click()}
            disabled={uploading}
            style={{
              display: "inline-flex",
              alignItems: "center",
              gap: "6px",
              padding: "7px 12px",
              borderRadius: "6px",
              border: "1px solid var(--color-accent)",
              backgroundColor: uploading ? "var(--color-bg-hover)" : "var(--color-accent)",
              color: uploading ? "var(--color-text-secondary)" : "#fff",
              cursor: uploading ? "default" : "pointer",
              fontSize: "13px",
              fontWeight: 600,
            }}
          >
            <Upload size={14} />
            {uploading ? t("common.saving", "Saving...") : t("settings.chooseBackgroundImage", "Choose background image")}
          </button>
          {backgroundImage && (
            <button
              type="button"
              aria-label={t("settings.removeBackgroundImage", "Remove background image")}
              onClick={handleRemoveBackgroundImage}
              disabled={uploading}
              style={{
                display: "inline-flex",
                alignItems: "center",
                gap: "6px",
                padding: "7px 12px",
                borderRadius: "6px",
                border: "1px solid var(--color-border)",
                backgroundColor: "transparent",
                color: "var(--color-text-secondary)",
                cursor: uploading ? "default" : "pointer",
                fontSize: "13px",
              }}
            >
              <Trash2 size={14} />
              {t("common.remove", "Remove")}
            </button>
          )}
        </div>

        {backgroundImage && (
          <>
            <div>
              <div style={{ fontSize: "12px", fontWeight: 600, marginBottom: "8px" }}>
                {t("settings.backgroundFit", "Image fit")}
              </div>
              <div role="group" aria-label={t("settings.backgroundFit", "Image fit")} style={{ display: "flex", flexWrap: "wrap", gap: "8px" }}>
                {BACKGROUND_FITS.map((fit) => {
                  const selected = backgroundImage.fit === fit.id;
                  return (
                    <button
                      key={fit.id}
                      type="button"
                      onClick={() => setBackgroundImageFit(fit.id)}
                      style={{
                        padding: "6px 10px",
                        borderRadius: "6px",
                        border: selected ? "2px solid var(--color-accent)" : "1px solid var(--color-border)",
                        backgroundColor: selected ? "var(--color-bg-hover)" : "transparent",
                        color: "var(--color-text-primary)",
                        cursor: "pointer",
                        fontSize: "12px",
                      }}
                    >
                      {t(fit.labelKey, fit.fallback)}
                    </button>
                  );
                })}
              </div>
            </div>
            <label style={{ display: "grid", gap: "8px", fontSize: "12px", fontWeight: 600 }}>
              <span>{t("settings.backgroundOpacity", "Image opacity")}</span>
              <input
                type="range"
                min={0.05}
                max={1}
                step={0.01}
                value={backgroundImage.opacity}
                onChange={(event) => setBackgroundImageOpacity(Number(event.currentTarget.value))}
                aria-label={t("settings.backgroundOpacity", "Image opacity")}
              />
            </label>
          </>
        )}

        {error && (
          <div role="alert" style={{ color: "#ef4444", fontSize: "12px" }}>
            {error}
          </div>
        )}
      </div>
    </div>
  );
}
