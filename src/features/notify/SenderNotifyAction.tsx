import { useMemo } from "react";
import { Bell, BellOff, Star } from "lucide-react";
import { useTranslation } from "react-i18next";
import ContextMenu, { useContextMenuState, type ContextMenuItem } from "@/components/ContextMenu";
import { useAccountsQuery } from "@/hooks/queries";
import { useToastStore } from "@/stores/toast.store";
import { useSenderPrefMutations, useSenderPrefs, type SenderPref } from "./api";
import "./notify.css";

/** The preference that applies to `address` on `accountId` (mirrors `Store::sender_pref_for`). */
export function effectiveSenderPref(prefs: SenderPref[], accountId: string, address: string): SenderPref | null {
  const normalized = address.trim().toLowerCase();
  const at = normalized.lastIndexOf("@");
  if (at < 0) return null;
  const domain = normalized.slice(at);
  for (const [scope, pattern] of [
    [accountId, normalized],
    [accountId, domain],
    ["", normalized],
    ["", domain],
  ] as const) {
    const match = prefs.find((pref) => pref.account_id === scope && pref.pattern === pattern);
    if (match) return match;
  }
  return null;
}

/** Small bell next to a sender: make them a VIP, mute them, or reset. */
export default function SenderNotifyAction({ accountId, address }: { accountId: string; address: string }) {
  const { t } = useTranslation();
  const menu = useContextMenuState();
  const addToast = useToastStore((s) => s.addToast);
  const { data: prefs = [] } = useSenderPrefs();
  const { data: accounts = [] } = useAccountsQuery();
  const { set, remove } = useSenderPrefMutations();
  const normalized = address.trim().toLowerCase();
  const current = useMemo(() => effectiveSenderPref(prefs, accountId, normalized), [prefs, accountId, normalized]);
  const isSelf = accounts.some((account) => account.email.trim().toLowerCase() === normalized);
  if (!normalized.includes("@") || isSelf) return null;

  const label = current?.kind === "vip"
    ? t("notify.senderIsVip", "VIP: always notifies")
    : current?.kind === "mute"
      ? t("notify.senderIsMuted", "Muted: never notifies")
      : t("notify.senderMenu", "Notifications for this sender");

  const fail = () => addToast({ message: t("notify.saveFailed", "Couldn't save notification settings"), type: "error" });
  const items: ContextMenuItem[] = [
    {
      id: "vip",
      label: t("notify.makeVip", "Always notify (VIP)"),
      icon: <Star size={14} />,
      disabled: current?.kind === "vip" && current.pattern === normalized,
      onSelect: () => set.mutate({ accountId: null, pattern: normalized, kind: "vip" }, { onError: fail }),
    },
    {
      id: "mute",
      label: t("notify.mute", "Mute this sender"),
      icon: <BellOff size={14} />,
      disabled: current?.kind === "mute" && current.pattern === normalized,
      onSelect: () => set.mutate({ accountId: null, pattern: normalized, kind: "mute" }, { onError: fail }),
    },
  ];
  if (current) {
    items.push({
      id: "reset",
      label: current.pattern.startsWith("@")
        ? t("notify.resetDomain", "Stop treating {{domain}} specially", { domain: current.pattern })
        : t("notify.reset", "Use normal notifications"),
      icon: <Bell size={14} />,
      separatorBefore: true,
      onSelect: () => remove.mutate({ accountId: current.account_id || null, pattern: current.pattern }, { onError: fail }),
    });
  }

  const Icon = current?.kind === "vip" ? Star : current?.kind === "mute" ? BellOff : Bell;
  return (
    <>
      <button
        type="button"
        className={`contact-address-action sender-notify-action${current ? ` is-${current.kind}` : ""}`}
        aria-label={label}
        title={label}
        aria-haspopup="menu"
        onClick={(event) => {
          const rect = event.currentTarget.getBoundingClientRect();
          menu.openAt(rect.left, rect.bottom + 4, event.currentTarget);
        }}
      >
        <Icon size={13} />
      </button>
      {menu.position && (
        <ContextMenu label={label} position={menu.position} items={items} onClose={menu.close} />
      )}
    </>
  );
}
