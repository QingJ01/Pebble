import { useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Image, Trash2, Upload } from "lucide-react";
import { useUIStore } from "@/stores/ui.store";
import type { BackgroundImageFit, HtmlMailDarkMode, Language, SidebarStyle, Theme } from "@/stores/ui.store";
import { backgroundImageUrl, deleteBackgroundImage, importBackgroundImage } from "@/lib/backgroundImage";
import { formatError } from "@/lib/extractErrorMessage";
import Button from "@/components/ui/Button";
import { OptionCard, OptionCardGroup, SettingsList, SettingsListRow, SettingsSection } from "./SettingsLayout";

const THEMES: { id: Theme; labelKey: string; descKey: string }[] = [
  { id: "light", labelKey: "settings.themeLight", descKey: "settings.themeLightDesc" },
  { id: "dark", labelKey: "settings.themeDark", descKey: "settings.themeDarkDesc" },
  { id: "system", labelKey: "settings.themeSystem", descKey: "settings.themeSystemDesc" },
];

const HTML_MAIL_DARK_MODES: { id: HtmlMailDarkMode; labelKey: string; label: string; descKey: string; desc: string }[] = [
  {
    id: "off",
    labelKey: "settings.htmlMailDarkOff",
    label: "Light card",
    descKey: "settings.htmlMailDarkOffDesc",
    desc: "Show HTML mail as its sender designed it",
  },
  {
    id: "auto",
    labelKey: "settings.htmlMailDarkAuto",
    label: "Darken simple mail",
    descKey: "settings.htmlMailDarkAutoDesc",
    desc: "Darken mail without designed backgrounds; keep the rest on a light card",
  },
  {
    id: "force",
    labelKey: "settings.htmlMailDarkForce",
    label: "Always dark",
    descKey: "settings.htmlMailDarkForceDesc",
    desc: "Invert every HTML mail; brand colors may shift",
  },
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
  return formatError(error, { fallbackKey: "settings.backgroundImageFailed", fallback: "Couldn't update the background image" });
}

export default function AppearanceTab() {
  const { t } = useTranslation();
  const fileInputRef = useRef<HTMLInputElement | null>(null);
  const [uploading, setUploading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const theme = useUIStore((s) => s.theme);
  const setTheme = useUIStore((s) => s.setTheme);
  const htmlMailDarkMode = useUIStore((s) => s.htmlMailDarkMode);
  const setHtmlMailDarkMode = useUIStore((s) => s.setHtmlMailDarkMode);
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
      <SettingsSection title={t("settings.theme")}>
        <OptionCardGroup label={t("settings.theme")} columns={3}>
          {THEMES.map((th) => (
            <OptionCard
              key={th.id}
              selected={theme === th.id}
              onClick={() => setTheme(th.id)}
              title={t(th.labelKey)}
              description={t(th.descKey)}
            />
          ))}
        </OptionCardGroup>
      </SettingsSection>

      <SettingsSection
        title={t("settings.htmlMailDark", "HTML mail in dark theme")}
        description={t("settings.htmlMailDarkDesc", "Plain-text mail always follows the theme.")}
      >
        <OptionCardGroup label={t("settings.htmlMailDark", "HTML mail in dark theme")} columns={3}>
          {HTML_MAIL_DARK_MODES.map((mode) => (
            <OptionCard
              key={mode.id}
              selected={htmlMailDarkMode === mode.id}
              onClick={() => setHtmlMailDarkMode(mode.id)}
              title={t(mode.labelKey, mode.label)}
              description={t(mode.descKey, mode.desc)}
            />
          ))}
        </OptionCardGroup>
      </SettingsSection>

      <SettingsSection title={t("settings.language")}>
        <OptionCardGroup label={t("settings.language")} columns={2}>
          {LANGUAGES.map((l) => (
            <OptionCard key={l.id} selected={language === l.id} onClick={() => setLanguage(l.id)} title={l.label} />
          ))}
        </OptionCardGroup>
      </SettingsSection>

      <SettingsSection title={t("settings.sidebarStyle", "Sidebar style")}>
        <OptionCardGroup label={t("settings.sidebarStyle", "Sidebar style")} columns={2}>
          {SIDEBAR_STYLES.map((style) => (
            <OptionCard
              key={style.id}
              selected={sidebarStyle === style.id}
              onClick={() => setSidebarStyle(style.id)}
              title={t(style.labelKey)}
              description={t(style.descKey)}
            />
          ))}
        </OptionCardGroup>
      </SettingsSection>

      <SettingsSection
        title={t("settings.backgroundImage", "Background image")}
        description={t("settings.backgroundImageDesc", "Stored locally in Pebble's app data folder.")}
      >
        <SettingsList>
          <SettingsListRow
            leading={
              <div
                aria-hidden="true"
                style={{
                  width: "64px",
                  height: "40px",
                  flexShrink: 0,
                  borderRadius: "var(--radius-md)",
                  border: "1px solid var(--color-border)",
                  backgroundColor: "var(--color-surface-sunken)",
                  backgroundImage: backgroundImage ? `url("${backgroundImageUrl(backgroundImage.path)}")` : undefined,
                  backgroundPosition: "center",
                  backgroundRepeat: backgroundImage?.fit === "repeat" ? "repeat" : "no-repeat",
                  backgroundSize: backgroundImage?.fit === "repeat" ? "auto" : backgroundImage?.fit ?? "cover",
                  display: "flex",
                  alignItems: "center",
                  justifyContent: "center",
                  color: "var(--color-text-tertiary)",
                  overflow: "hidden",
                }}
              >
                {!backgroundImage && <Image size={18} />}
              </div>
            }
            title={backgroundImage?.filename ?? t("settings.noBackgroundImage", "No background image")}
            actions={
              <>
                <Button
                  size="sm"
                  onClick={() => fileInputRef.current?.click()}
                  loading={uploading}
                  icon={<Upload size={14} />}
                >
                  {uploading ? t("common.saving", "Saving...") : t("settings.chooseBackgroundImage", "Choose background image")}
                </Button>
                {backgroundImage && (
                  <Button
                    size="sm"
                    variant="danger-ghost"
                    aria-label={t("settings.removeBackgroundImage", "Remove background image")}
                    onClick={handleRemoveBackgroundImage}
                    disabled={uploading}
                    icon={<Trash2 size={14} />}
                  >
                    {t("common.remove", "Remove")}
                  </Button>
                )}
              </>
            }
          />
        </SettingsList>

        <input
          ref={fileInputRef}
          aria-label={t("settings.chooseBackgroundImage", "Choose background image")}
          type="file"
          accept="image/png,image/jpeg,image/webp,image/gif"
          onChange={handleBackgroundFileChange}
          style={{ display: "none" }}
        />

        {backgroundImage && (
          <>
            <div className="field">
              <span className="field__label">{t("settings.backgroundFit", "Image fit")}</span>
              <OptionCardGroup label={t("settings.backgroundFit", "Image fit")} columns={3}>
                {BACKGROUND_FITS.map((fit) => (
                  <OptionCard
                    key={fit.id}
                    selected={backgroundImage.fit === fit.id}
                    onClick={() => setBackgroundImageFit(fit.id)}
                    title={t(fit.labelKey, fit.fallback)}
                  />
                ))}
              </OptionCardGroup>
            </div>
            <label className="field">
              <span className="field__label">{t("settings.backgroundOpacity", "Image opacity")}</span>
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
          <p role="alert" className="settings-error">
            {error}
          </p>
        )}
      </SettingsSection>
    </div>
  );
}
