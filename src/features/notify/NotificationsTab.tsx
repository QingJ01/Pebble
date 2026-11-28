import { useCallback, useState, type FormEvent } from "react";
import { useTranslation } from "react-i18next";
import { useQueryClient } from "@tanstack/react-query";
import Button from "@/components/ui/Button";
import Checkbox from "@/components/ui/Checkbox";
import Input from "@/components/ui/Input";
import Select from "@/components/ui/Select";
import { useAccountsQuery } from "@/hooks/queries";
import { showTestNotification } from "@/lib/api";
import { useToastStore } from "@/stores/toast.store";
import { useUIStore } from "@/stores/ui.store";
import {
  OptionCard,
  OptionCardGroup,
  SettingsEmpty,
  SettingsList,
  SettingsListRow,
  SettingsSection,
} from "@/features/settings/SettingsLayout";
import {
  DEFAULT_QUIET_HOURS,
  minutesToTime,
  notifyKeys,
  resetAccountNotifyPrefs,
  timeToMinutes,
  useNotificationLog,
  useNotifyChangedInvalidation,
  useNotifySettings,
  useSaveNotifyPrefs,
  useSenderPrefMutations,
  useSenderPrefs,
  type NotifyMode,
  type NotifyPrefs,
} from "./api";
import { reasonLabel, tierLabel } from "./labels";
import "./notify.css";

const MODES: Array<{ mode: NotifyMode; titleKey: string; title: string; descKey: string; desc: string }> = [
  {
    mode: "all",
    titleKey: "notify.modeAll",
    title: "All new mail",
    descKey: "notify.modeAllDesc",
    desc: "Every new message in the inbox notifies, except spam, muted senders and mail your rules file away.",
  },
  {
    mode: "smart",
    titleKey: "notify.modeSmart",
    title: "Smart",
    descKey: "notify.modeSmartDesc",
    desc: "People you've written to, replies to you, followed threads and VIPs notify. Newsletters and unknown senders go to the digest.",
  },
  {
    mode: "important_only",
    titleKey: "notify.modeImportant",
    title: "Important only",
    descKey: "notify.modeImportantDesc",
    desc: "Only followed threads, replies to your mail and VIPs notify. Everything else goes to the digest.",
  },
];

export default function NotificationsTab() {
  const { t } = useTranslation();
  useNotifyChangedInvalidation();
  const addToast = useToastStore((s) => s.addToast);
  const notificationsEnabled = useUIStore((s) => s.notificationsEnabled);
  const setNotificationsEnabled = useUIStore((s) => s.setNotificationsEnabled);
  const [testing, setTesting] = useState(false);
  const { data: settings } = useNotifySettings();
  const { data: accounts = [] } = useAccountsQuery();
  const save = useSaveNotifyPrefs();
  const queryClient = useQueryClient();

  const defaults = settings?.defaults;
  const saveDefaults = useCallback(
    (patch: Partial<NotifyPrefs>) => {
      if (!defaults) return;
      save.mutate(
        { accountId: null, prefs: { ...defaults, ...patch } },
        { onError: () => addToast({ message: t("notify.saveFailed", "Couldn't save notification settings"), type: "error" }) },
      );
    },
    [defaults, save, addToast, t],
  );

  async function handleTest() {
    setTesting(true);
    try {
      await showTestNotification();
      addToast({ message: t("settings.testNotificationSent", "Test notification sent"), type: "success" });
    } catch {
      addToast({ message: t("settings.testNotificationFailed", "Failed to send test notification"), type: "error" });
    } finally {
      setTesting(false);
    }
  }

  return (
    <div>
      <SettingsSection title={t("notify.desktopTitle", "Desktop notifications")}>
        <Checkbox
          checked={notificationsEnabled}
          onChange={() => setNotificationsEnabled(!notificationsEnabled)}
          label={t("settings.enableNotifications", "Enable desktop notifications")}
        />
        <div className="settings-actions">
          <Button onClick={handleTest} disabled={!notificationsEnabled} loading={testing}>
            {t("settings.testNotification", "Send test notification")}
          </Button>
        </div>
      </SettingsSection>

      <SettingsSection
        title={t("notify.whenTitle", "When to notify")}
        description={t(
          "notify.whenDesc",
          "Spam, muted senders, your own mail and mail a rule marks read or files away never notify. Mail that doesn't notify is still in your inbox.",
        )}
      >
        <OptionCardGroup label={t("notify.whenTitle", "When to notify")} columns={3}>
          {MODES.map((option) => (
            <OptionCard
              key={option.mode}
              selected={defaults?.mode === option.mode}
              disabled={!defaults}
              onClick={() => saveDefaults({ mode: option.mode })}
              title={t(option.titleKey, option.title)}
              description={t(option.descKey, option.desc)}
            />
          ))}
        </OptionCardGroup>
        {accounts.length > 1 && defaults && (
          <SettingsList className="notify-account-list">
            {accounts.map((account) => {
              const own = settings?.accounts[account.id];
              return (
                <SettingsListRow
                  key={account.id}
                  title={account.account_label || account.email}
                  meta={account.account_label ? account.email : undefined}
                  actions={
                    <Select
                      size="sm"
                      aria-label={t("notify.accountMode", "Notifications for {{account}}", { account: account.email })}
                      value={own?.mode ?? "default"}
                      onChange={(event) => {
                        const value = event.target.value;
                        if (value === "default") {
                          void resetAccountNotifyPrefs(account.id).finally(() =>
                            queryClient.invalidateQueries({ queryKey: notifyKeys.settings }),
                          );
                        } else {
                          save.mutate({ accountId: account.id, prefs: { ...(own ?? defaults), mode: value as NotifyMode } });
                        }
                      }}
                    >
                      <option value="default">{t("notify.followDefault", "Same as above")}</option>
                      {MODES.map((option) => (
                        <option key={option.mode} value={option.mode}>{t(option.titleKey, option.title)}</option>
                      ))}
                    </Select>
                  }
                />
              );
            })}
          </SettingsList>
        )}
      </SettingsSection>

      {defaults && <QuietHoursSection prefs={defaults} onChange={saveDefaults} />}
      <SenderPrefsSection />
      <DecisionLogSection />
    </div>
  );
}

function QuietHoursSection({ prefs, onChange }: { prefs: NotifyPrefs; onChange: (patch: Partial<NotifyPrefs>) => void }) {
  const { t } = useTranslation();
  const quiet = prefs.quietHours;
  const update = (patch: Partial<typeof DEFAULT_QUIET_HOURS>) => onChange({ quietHours: { ...(quiet ?? DEFAULT_QUIET_HOURS), ...patch } });
  return (
    <SettingsSection
      title={t("notify.quietTitle", "Quiet hours")}
      description={t("notify.quietDesc", "During quiet hours, notifications wait in the digest.")}
    >
      <Checkbox
        checked={!!quiet}
        onChange={() => onChange({ quietHours: quiet ? null : DEFAULT_QUIET_HOURS })}
        label={t("notify.quietEnable", "Use quiet hours")}
      />
      {quiet && (
        <>
          <div className="notify-quiet-range">
            <label className="field">
              <span className="field__label">{t("notify.quietFrom", "From")}</span>
              <Input
                type="time"
                value={minutesToTime(quiet.startMinute)}
                onChange={(e) => {
                  const minutes = timeToMinutes(e.target.value);
                  if (minutes !== null) update({ startMinute: minutes });
                }}
              />
            </label>
            <label className="field">
              <span className="field__label">{t("notify.quietTo", "To")}</span>
              <Input
                type="time"
                value={minutesToTime(quiet.endMinute)}
                onChange={(e) => {
                  const minutes = timeToMinutes(e.target.value);
                  if (minutes !== null) update({ endMinute: minutes });
                }}
              />
            </label>
          </div>
          <div className="settings-checks">
            <Checkbox
              checked={quiet.allowWatch}
              onChange={() => update({ allowWatch: !quiet.allowWatch })}
              label={t("notify.quietAllowWatch", "Still notify for replies to your mail and followed threads")}
            />
            <Checkbox
              checked={quiet.allowVip}
              onChange={() => update({ allowVip: !quiet.allowVip })}
              label={t("notify.quietAllowVip", "Still notify for VIPs")}
            />
          </div>
        </>
      )}
    </SettingsSection>
  );
}

function SenderPrefsSection() {
  const { t } = useTranslation();
  const addToast = useToastStore((s) => s.addToast);
  const { data: prefs = [] } = useSenderPrefs();
  const { data: accounts = [] } = useAccountsQuery();
  const { set, remove } = useSenderPrefMutations();
  const [pattern, setPattern] = useState("");
  const [kind, setKind] = useState<"vip" | "mute">("vip");
  const [scope, setScope] = useState("");

  function handleAdd(event: FormEvent) {
    event.preventDefault();
    if (!pattern.trim()) return;
    set.mutate(
      { accountId: scope || null, pattern, kind },
      {
        onSuccess: () => setPattern(""),
        onError: () => addToast({ message: t("notify.senderInvalid", "Enter an email address or @domain"), type: "error" }),
      },
    );
  }

  const accountName = (id: string) => {
    if (!id) return t("notify.allAccounts", "All accounts");
    const account = accounts.find((a) => a.id === id);
    return account ? account.account_label || account.email : id;
  };

  return (
    <SettingsSection
      title={t("notify.sendersTitle", "VIPs and muted senders")}
      description={t("notify.sendersDesc", "VIPs always notify. Muted senders never notify; their mail still arrives. Use @domain for a whole domain.")}
    >
      <form className="notify-sender-form" onSubmit={handleAdd}>
        <Input
          value={pattern}
          onChange={(e) => setPattern(e.target.value)}
          placeholder={t("notify.senderPlaceholder", "name@example.com or @example.com")}
          aria-label={t("notify.senderPattern", "Address or domain")}
        />
        <Select value={kind} onChange={(e) => setKind(e.target.value as "vip" | "mute")} aria-label={t("notify.senderKind", "Type")}>
          <option value="vip">{t("notify.kindVip", "VIP")}</option>
          <option value="mute">{t("notify.kindMute", "Muted")}</option>
        </Select>
        {accounts.length > 1 && (
          <Select value={scope} onChange={(e) => setScope(e.target.value)} aria-label={t("notify.senderScope", "Applies to")}>
            <option value="">{t("notify.allAccounts", "All accounts")}</option>
            {accounts.map((account) => (
              <option key={account.id} value={account.id}>{account.account_label || account.email}</option>
            ))}
          </Select>
        )}
        <Button type="submit" disabled={!pattern.trim()} loading={set.isPending}>{t("notify.add", "Add")}</Button>
      </form>
      {prefs.length === 0 ? (
        <SettingsEmpty>{t("notify.sendersEmpty", "No VIPs or muted senders yet. You can also add them from a message's sender menu.")}</SettingsEmpty>
      ) : (
        <SettingsList>
          {prefs.map((pref) => (
            <SettingsListRow
              key={`${pref.account_id}:${pref.pattern}`}
              leading={<span className={`notify-chip notify-chip--${pref.kind}`}>{pref.kind === "vip" ? t("notify.kindVip", "VIP") : t("notify.kindMute", "Muted")}</span>}
              title={pref.pattern}
              meta={accountName(pref.account_id)}
              actions={
                <Button size="sm" variant="ghost" onClick={() => remove.mutate({ accountId: pref.account_id || null, pattern: pref.pattern })}>
                  {t("common.remove", "Remove")}
                </Button>
              }
            />
          ))}
        </SettingsList>
      )}
    </SettingsSection>
  );
}

function DecisionLogSection() {
  const { t } = useTranslation();
  const { data: log = [] } = useNotificationLog(50);
  return (
    <SettingsSection
      title={t("notify.logTitle", "Recent decisions")}
      description={t("notify.logDesc", "Why each new message did or didn't notify. Only the decision is kept, not the message text.")}
    >
      {log.length === 0 ? (
        <SettingsEmpty>{t("notify.logEmpty", "Nothing yet. New mail will show up here.")}</SettingsEmpty>
      ) : (
        <SettingsList className="notify-log">
          {log.map((entry) => (
            <SettingsListRow
              key={`${entry.accountId}:${entry.messageId}`}
              leading={<span className={`notify-chip notify-chip--${entry.tier}`}>{tierLabel(t, entry.tier)}</span>}
              title={entry.subject || t("inbox.noSubject", "(no subject)")}
              meta={
                <>
                  {entry.fromName || entry.fromAddress} · {reasonLabel(t, entry.reason)}
                  {entry.why && <span className="notify-why"> · {entry.why}</span>}
                </>
              }
            />
          ))}
        </SettingsList>
      )}
    </SettingsSection>
  );
}
