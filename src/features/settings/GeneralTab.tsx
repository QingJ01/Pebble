import { useCallback, useEffect, useState } from "react";
import ScheduledSendsView from "@/features/compose/ScheduledSendsView";
import { useTranslation } from "react-i18next";
import {
  showTestNotification,
  openDefaultMailSettings,
  getAutostartEnabled,
  setAutostartEnabled,
} from "@/lib/api";
import { useToastStore } from "@/stores/toast.store";
import { useUIStore, type RealtimePreference } from "@/stores/ui.store";
import Button from "@/components/ui/Button";
import Checkbox from "@/components/ui/Checkbox";
import { OptionCard, OptionCardGroup, SettingsSection } from "./SettingsLayout";

const REALTIME_OPTIONS: Array<{
  mode: RealtimePreference;
  labelKey: string;
  fallback: string;
  descriptionKey: string;
  descriptionFallback: string;
}> = [
  {
    mode: "realtime",
    labelKey: "settings.realtimeModeRealtime",
    fallback: "Realtime (recommended)",
    descriptionKey: "settings.realtimeModeRealtimeDesc",
    descriptionFallback: "IMAP uses IDLE push when supported. Other providers check about every 3 seconds while you are active.",
  },
  {
    mode: "balanced",
    labelKey: "settings.realtimeModeBalanced",
    fallback: "Balanced",
    descriptionKey: "settings.realtimeModeBalancedDesc",
    descriptionFallback: "Checks about every 15 seconds while you are active.",
  },
  {
    mode: "battery",
    labelKey: "settings.realtimeModeBattery",
    fallback: "Battery saver",
    descriptionKey: "settings.realtimeModeBatteryDesc",
    descriptionFallback: "Checks about every 60 seconds while you are active and slows down in the background.",
  },
  {
    mode: "manual",
    labelKey: "settings.realtimeModeManual",
    fallback: "Manual only",
    descriptionKey: "settings.realtimeModeManualDesc",
    descriptionFallback: "Stops background checks. Use Sync now to run a single pass.",
  },
];

export default function GeneralTab() {
  const { t } = useTranslation();
  const addToast = useToastStore((s) => s.addToast);
  const [testingNotification, setTestingNotification] = useState(false);
  const realtimeMode = useUIStore((s) => s.realtimeMode);
  const setRealtimeMode = useUIStore((s) => s.setRealtimeMode);
  const notificationsEnabled = useUIStore((s) => s.notificationsEnabled);
  const setNotificationsEnabled = useUIStore((s) => s.setNotificationsEnabled);
  const keepRunningInBackground = useUIStore((s) => s.keepRunningInBackground);
  const setKeepRunningInBackground = useUIStore((s) => s.setKeepRunningInBackground);
  const startHiddenToTray = useUIStore((s) => s.startHiddenToTray);
  const setStartHiddenToTray = useUIStore((s) => s.setStartHiddenToTray);

  const toggleNotifications = useCallback(() => {
    setNotificationsEnabled(!notificationsEnabled);
  }, [notificationsEnabled, setNotificationsEnabled]);

  const handleTestNotification = useCallback(async () => {
    setTestingNotification(true);
    try {
      await showTestNotification();
      addToast({
        message: t("settings.testNotificationSent", "Test notification sent"),
        type: "success",
      });
    } catch {
      addToast({
        message: t("settings.testNotificationFailed", "Failed to send test notification"),
        type: "error",
      });
    } finally {
      setTestingNotification(false);
    }
  }, [addToast, t]);

  const quitOnClose = !keepRunningInBackground;
  const toggleQuitOnClose = useCallback(() => {
    setKeepRunningInBackground(quitOnClose);
  }, [quitOnClose, setKeepRunningInBackground]);

  const toggleStartHiddenToTray = useCallback(() => {
    setStartHiddenToTray(!startHiddenToTray);
  }, [setStartHiddenToTray, startHiddenToTray]);

  // Launch-at-startup state lives in the OS (registry / LaunchAgent / .desktop),
  // not in app storage, so read the real value from the backend on mount.
  const [launchAtStartup, setLaunchAtStartup] = useState(false);

  useEffect(() => {
    let active = true;
    getAutostartEnabled()
      .then((enabled) => {
        if (active) setLaunchAtStartup(enabled);
      })
      .catch((err) => {
        console.warn("Failed to read launch-at-startup state", err);
      });
    return () => {
      active = false;
    };
  }, []);

  const toggleLaunchAtStartup = useCallback(async () => {
    const next = !launchAtStartup;
    setLaunchAtStartup(next); // optimistic; revert below if the OS rejects it
    try {
      await setAutostartEnabled(next);
    } catch {
      setLaunchAtStartup(!next);
      addToast({
        message: t("settings.autostartFailed", "Failed to update launch at startup"),
        type: "error",
      });
    }
  }, [launchAtStartup, addToast, t]);

  const showUnreadCount = useUIStore((s) => s.showFolderUnreadCount);
  const setShowUnreadCount = useUIStore((s) => s.setShowFolderUnreadCount);

  const toggleUnreadCount = useCallback(() => {
    setShowUnreadCount(!showUnreadCount);
  }, [showUnreadCount, setShowUnreadCount]);

  return (
    <div>
      <SettingsSection
        title={t("settings.realtimeMode", "Realtime Mode")}
        description={t("settings.realtimeModeDesc", "Choose how aggressively Pebble checks for new mail.")}
      >
        <OptionCardGroup label={t("settings.realtimeMode", "Realtime Mode")} columns={2}>
          {REALTIME_OPTIONS.map((option) => {
            const label = t(option.labelKey, option.fallback);
            return (
              <OptionCard
                key={option.mode}
                aria-label={label}
                selected={realtimeMode === option.mode}
                onClick={() => setRealtimeMode(option.mode)}
                title={label}
                description={t(option.descriptionKey, option.descriptionFallback)}
              />
            );
          })}
        </OptionCardGroup>
      </SettingsSection>

      <SettingsSection title={t("settings.notifications")}>
        <Checkbox
          checked={notificationsEnabled}
          onChange={toggleNotifications}
          label={t("settings.enableNotifications")}
        />
        <div className="settings-actions">
          <Button
            aria-label={t("settings.testNotification", "Send test notification")}
            onClick={handleTestNotification}
            disabled={!notificationsEnabled}
            loading={testingNotification}
          >
            {testingNotification
              ? t("settings.testNotificationSending", "Sending test notification...")
              : t("settings.testNotification", "Send test notification")}
          </Button>
        </div>
      </SettingsSection>

      <SettingsSection title={t("settings.startupBehavior", "Startup Behavior")}>
        <div className="settings-checks">
          <Checkbox
            checked={launchAtStartup}
            onChange={toggleLaunchAtStartup}
            label={t("settings.launchAtStartup", "Launch Pebble at system startup")}
          />
          <Checkbox
            checked={startHiddenToTray}
            onChange={toggleStartHiddenToTray}
            label={t("settings.startHiddenToTray", "Start hidden to tray")}
          />
        </div>
      </SettingsSection>

      <SettingsSection title={t("settings.closeBehavior", "Close Behavior")}>
        <Checkbox
          checked={quitOnClose}
          onChange={toggleQuitOnClose}
          label={t("settings.quitOnClose", "Quit app when window is closed")}
        />
      </SettingsSection>

      <SettingsSection title={t("settings.folderCounts", "Folder Counts")}>
        <Checkbox
          checked={showUnreadCount}
          onChange={toggleUnreadCount}
          label={t("settings.showUnreadCount", "Show unread count badges in sidebar")}
        />
      </SettingsSection>

      <ScheduledSendsView variant="section" />

      {navigator.userAgent.includes("Windows") && (
        <SettingsSection
          title={t("settings.defaultMailClient", "Default Email Client")}
          description={t("settings.setDefaultMailClientHint", "Opens Windows Default Apps settings where you can select Pebble for email.")}
        >
          <div className="settings-actions">
            <Button onClick={() => openDefaultMailSettings()}>
              {t("settings.setDefaultMailClient", "Set Pebble as default email app")}
            </Button>
          </div>
        </SettingsSection>
      )}
    </div>
  );
}
