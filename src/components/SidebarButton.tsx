import type { CSSProperties, MouseEvent, ReactNode } from "react";

/**
 * One sidebar navigation entry. Hover/active styling lives in CSS
 * (`.sidebar-btn` in src/app/shell.css). In the collapsed icon rail the label
 * becomes the accessible name and tooltip; expanded, a tooltip is added only
 * when the label is actually truncated.
 */
export default function SidebarButton({
  icon, label, tooltip, badge, isActive, collapsed, indent = false, disabled, onClick, style, dropFolderId,
}: {
  icon: ReactNode;
  label: string;
  /** Hover text shown even when the label fits, e.g. a nested folder's full path */
  tooltip?: string;
  badge?: number;
  isActive: boolean;
  collapsed: boolean;
  indent?: boolean;
  disabled?: boolean;
  onClick: () => void;
  style?: CSSProperties;
  /** Makes this entry a drop target for dragged mail (see lib/mailDrag). */
  dropFolderId?: string;
}) {
  function syncTruncationTitle(event: MouseEvent<HTMLButtonElement>) {
    if (collapsed || tooltip) return;
    const labelEl = event.currentTarget.querySelector<HTMLElement>(".sidebar-btn__label");
    if (!labelEl) return;
    if (labelEl.scrollWidth > labelEl.clientWidth) event.currentTarget.title = label;
    else event.currentTarget.removeAttribute("title");
  }

  return (
    <button
      type="button"
      onClick={onClick}
      aria-label={collapsed ? label : undefined}
      aria-current={isActive ? "page" : undefined}
      title={collapsed ? label : tooltip}
      disabled={disabled}
      onMouseEnter={syncTruncationTitle}
      onFocus={(event) => syncTruncationTitle(event as unknown as MouseEvent<HTMLButtonElement>)}
      className={[
        "sidebar-btn",
        collapsed ? "sidebar-btn--rail" : null,
        indent && !collapsed ? "sidebar-btn--indent" : null,
        isActive ? "is-active" : null,
      ].filter(Boolean).join(" ")}
      style={style}
      data-drop-folder-id={dropFolderId}
    >
      <span aria-hidden="true" className="sidebar-btn__icon">{icon}</span>
      {!collapsed && <span className="sidebar-btn__label">{label}</span>}
      {!collapsed && badge != null && badge > 0 && (
        <span className="sidebar-btn__badge">{badge}</span>
      )}
      {collapsed && badge != null && badge > 0 && (
        <span className="sidebar-btn__dot" aria-hidden="true" />
      )}
    </button>
  );
}
