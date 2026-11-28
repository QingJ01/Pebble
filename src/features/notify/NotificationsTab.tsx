import { useCallback, useState, type FormEvent } from "react";
import { useTranslation } from "react-i18next";
import { useQueryClient } from "@tanstack/react-query";
import Button from "@/components/ui/Button";
import Checkbox from "@/components/ui/Checkbox";
import Input from "@/components/ui/Input";
import Select from "@/components/ui/Select";
import { useAccountsQuery } from "@/hooks/queries";
import { showTestNotification } from "@/lib/api";
import { toastError, useToastStore } from "@/stores/toast.store";
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
import { NotifyFeedback } from "./NotifyFeedback";
import { useAgentModel } from "@/features/agent/model-api";
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

      {defaults && settings && <ModelAssistSection defaults={defaults} own={settings.accounts} />}
      {defaults && <QuietHoursSection prefs={defaults} onChange={saveDefaults} />}
      {defaults && <ProactiveSection prefs={defaults} onChange={saveDefaults} />}
      <SenderPrefsSection />
      <DecisionLogSection />
    </div>
  );
}

/** Let the AI model decide mail the rules can't place (#107 §2.7). */
function ModelAssistSection({ defaults, own }: { defaults: NotifyPrefs; own: Record<string, NotifyPrefs> }) {
  const { t } = useTranslation();
  const { data: model } = useAgentModel();
  const { data: accounts = [] } = useAccountsQuery();
  const save = useSaveNotifyPrefs();
  const setSettingsTab = useUIStore((s) => s.setSettingsTab);
  const prefsOf = (id: string) => own[id] ?? defaults;
  const assisted = accounts.filter((account) => prefsOf(account.id).modelAssist);
  const update = (id: string, patch: Partial<NotifyPrefs>) =>
    save.mutate(
      { accountId: id, prefs: { ...prefsOf(id), ...patch } },
      { onError: (error) => toastError(error, "notify.saveFailed") },
    );
  const shared = assisted[0] ? prefsOf(assisted[0].id) : defaults;
  return (
    <SettingsSection
      title={t("notify.aiTitle", "AI help")}
      description={t(
        "notify.aiDesc",
        "For mail the rules can't place, the AI model decides whether it's worth a notification and says why. It can only choose between notifying and the digest, never hide mail. If the model is slow or unavailable, the rules decide.",
      )}
    >
      {!model?.configured ? (
        <div className="notify-ai__setup">
          <span>{t("notify.aiNeedsModel", "Set up an AI model first.")}</span>
          <Button size="sm" onClick={() => setSettingsTab("assistant")}>{t("agent.setupAction", "Set up a model")}</Button>
        </div>
      ) : (
        <>
          <p className={`notify-ai__privacy${model.local ? "" : " is-cloud"}`}>
            {shared.modelSendBody
              ? t("notify.aiSendsBody", "New mail from accounts you turn on is sent automatically to {{endpoint}}: sender, subject, whether you're in To, and the start of the text with codes masked.", { endpoint: model.endpoint })
              : t("notify.aiSendsMeta", "New mail from accounts you turn on is sent automatically to {{endpoint}}: sender, subject and whether you're in To. No message text.", { endpoint: model.endpoint })}
            {!model.local && " " + t("notify.aiCloud", "This is a cloud service; a local model keeps your mail on this device.")}
          </p>
          <SettingsList className="notify-account-list">
            {accounts.map((account) => {
              const prefs = prefsOf(account.id);
              const gmail = account.provider === "gmail";
              return (
                <SettingsListRow
                  key={account.id}
                  title={account.account_label || account.email}
                  meta={
                    gmail
                      ? t("notify.aiGmail", "Not available for Gmail accounts yet")
                      : prefs.modelAssist && prefs.mode === "all"
                        ? t("notify.aiModeAll", "Works with Smart or Important only; this account notifies for all mail")
                        : account.account_label ? account.email : undefined
                  }
                  actions={
                    <Checkbox
                      checked={prefs.modelAssist && !gmail}
                      disabled={gmail || save.isPending}
                      onChange={() => update(account.id, { modelAssist: !prefs.modelAssist })}
                      label={t("notify.aiUse", "Use AI")}
                      aria-label={t("notify.aiFor", "AI help for {{account}}", { account: account.email })}
                    />
                  }
                />
              );
            })}
          </SettingsList>
          {assisted.length > 0 && (
            <div className="notify-ai__options">
              <Checkbox
                checked={shared.modelSendBody}
                onChange={() => assisted.forEach((account) => update(account.id, { modelSendBody: !shared.modelSendBody }))}
                label={t("notify.aiBody", "Include the start of the message text (better decisions)")}
              />
              <Checkbox
                checked={shared.modelReviewsBulk}
                onChange={() => assisted.forEach((account) => update(account.id, { modelReviewsBulk: !shared.modelReviewsBulk }))}
                label={t("notify.aiBulk", "Also check newsletters and automated mail")}
              />
            </div>
          )}
        </>
      )}
    </SettingsSection>
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

function ProactiveSection({ prefs, onChange }: { prefs: NotifyPrefs; onChange: (patch: Partial<NotifyPrefs>) => void }) {
  const { t } = useTranslation();
  return (
    <SettingsSection
      title={t("notify.proactiveTitle", "Pebble keeps an eye out")}
      description={t(
        "notify.proactiveDesc",
        "Pebble can speak up on its own: a morning briefing, mail that's waiting for your reply, and follow-ups that got no answer. Everything shows in the assistant panel; desktop notifications follow the settings above.",
      )}
    >
      <div className="notify-proactive-row">
        <Checkbox
          checked={prefs.briefingEnabled}
          onChange={() => onChange({ briefingEnabled: !prefs.briefingEnabled })}
          label={t("notify.briefingEnable", "Daily briefing at")}
        />
        <Input
          type="time"
          className="notify-proactive-time"
          aria-label={t("notify.briefingTime", "Briefing time")}
          disabled={!prefs.briefingEnabled}
          value={minutesToTime(prefs.briefingMinute)}
          onChange={(e) => {
            const minutes = timeToMinutes(e.target.value);
            if (minutes !== null) onChange({ briefingMinute: minutes });
          }}
        />
      </div>
      <div className="notify-proactive-row">
        <Checkbox
          checked={prefs.awaitingReplyEnabled}
          onChange={() => onChange({ awaitingReplyEnabled: !prefs.awaitingReplyEnabled })}
          label={t("notify.awaitingEnable", "Remind me about mail waiting for my reply after")}
        />
        <Select
          size="sm"
          aria-label={t("notify.awaitingAfter", "Remind after")}
          disabled={!prefs.awaitingReplyEnabled}
          value={String(prefs.awaitingReplyHours)}
          onChange={(e) => onChange({ awaitingReplyHours: Number(e.target.value) })}
        >
          {[12, 24, 48, 72].map((hours) => (
            <option key={hours} value={hours}>
              {hours < 24 ? t("notify.hours", "{{n}} hours", { n: hours }) : t("notify.days", "{{n}} days", { n: hours / 24 })}
            </option>
          ))}
        </Select>
      </div>
      <div className="notify-proactive-row">
        <span>{t("notify.followUpDefault", "When I follow up on sent mail, remind me after")}</span>
        <Select
          size="sm"
          aria-label={t("notify.followUpDays", "Follow-up reminder")}
          value={String(prefs.followUpDays)}
          onChange={(e) => onChange({ followUpDays: Number(e.target.value) })}
        >
          {[1, 2, 3, 5, 7].map((days) => (
            <option key={days} value={days}>{t("notify.days", "{{n}} days", { n: days })}</option>
          ))}
        </Select>
      </div>
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
                  {entry.why && <span className="notify-why"> · ✦ {entry.why}</span>}
                </>
              }
              actions={<NotifyFeedback entry={entry} />}
            />
          ))}
        </SettingsList>
      )}
    </SettingsSection>
  );
}
