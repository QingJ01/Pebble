/**
 * Settings layout pieces (styles: "Settings" block in src/styles/index.css).
 *
 *   <SettingsSection title="Notifications" description="…" actions={<Button size="sm">…</Button>}>
 *     <Checkbox label="…" />
 *   </SettingsSection>
 *
 *   <OptionCardGroup label="Theme" columns={3}>
 *     <OptionCard selected={theme === "light"} title="Light" description="…" onClick={…} />
 *   </OptionCardGroup>
 *
 *   <SettingsList>
 *     <SettingsListRow title="a@b.c" meta="IMAP" actions={<Button size="sm">Edit</Button>} />
 *   </SettingsList>
 *
 * Every settings tab is a stack of SettingsSections (32px apart) under the
 * page header SettingsView renders; never add ad-hoc margins between them.
 */
import type { ComponentPropsWithoutRef, CSSProperties, ReactNode } from "react";

export function SettingsSection({
  title,
  description,
  actions,
  children,
  className,
  headingId,
  ...rest
}: {
  title: ReactNode;
  description?: ReactNode;
  actions?: ReactNode;
  children?: ReactNode;
  headingId?: string;
} & Omit<ComponentPropsWithoutRef<"section">, "title">) {
  return (
    <section {...rest} className={["settings-section", className].filter(Boolean).join(" ")}>
      <div className="settings-section__header">
        <div className="settings-section__heading">
          <h3 id={headingId} className="settings-section__title">{title}</h3>
          {description && <p className="settings-section__description">{description}</p>}
        </div>
        {actions && <div className="settings-section__actions">{actions}</div>}
      </div>
      {children !== undefined && children !== null && children !== false && (
        <div className="settings-section__body">{children}</div>
      )}
    </section>
  );
}

/** Equal-height grid of selectable cards (theme, language, realtime mode…). */
export function OptionCardGroup({
  label,
  columns = 3,
  children,
}: {
  label: string;
  columns?: 2 | 3 | 4;
  children: ReactNode;
}) {
  return (
    <div
      role="group"
      aria-label={label}
      className="option-card-grid"
      style={{ "--option-columns": columns } as CSSProperties}
    >
      {children}
    </div>
  );
}

export function OptionCard({
  selected,
  title,
  description,
  className,
  ...rest
}: {
  selected: boolean;
  title: ReactNode;
  description?: ReactNode;
} & Omit<ComponentPropsWithoutRef<"button">, "title" | "children" | "aria-pressed">) {
  return (
    <button
      type="button"
      {...rest}
      aria-pressed={selected}
      className={["option-card", className].filter(Boolean).join(" ")}
    >
      <span className="option-card__title">{title}</span>
      {description && <span className="option-card__description">{description}</span>}
    </button>
  );
}

/** Bordered list of rows (accounts, rules, trusted senders, scheduled sends…). */
export function SettingsList({ className, ...rest }: ComponentPropsWithoutRef<"div">) {
  return <div {...rest} className={["settings-list", className].filter(Boolean).join(" ")} />;
}

export function SettingsListRow({
  leading,
  title,
  meta,
  actions,
  children,
  className,
  ...rest
}: {
  leading?: ReactNode;
  title?: ReactNode;
  meta?: ReactNode;
  actions?: ReactNode;
  children?: ReactNode;
} & Omit<ComponentPropsWithoutRef<"div">, "title">) {
  return (
    <div {...rest} className={["settings-list__row", className].filter(Boolean).join(" ")}>
      {leading}
      <div className="settings-list__main">
        {title !== undefined && <div className="settings-list__title">{title}</div>}
        {meta !== undefined && meta !== null && meta !== false && <div className="settings-list__meta">{meta}</div>}
        {children}
      </div>
      {actions && <div className="settings-list__actions">{actions}</div>}
    </div>
  );
}

/** Empty-state line for a section or list. */
export function SettingsEmpty({ children, ...rest }: ComponentPropsWithoutRef<"p">) {
  return <p {...rest} className="settings-empty">{children}</p>;
}

export type NoticeTone = "neutral" | "info" | "success" | "warning" | "danger";

/** Inline status/notice box. */
export function SettingsNotice({
  tone = "neutral",
  className,
  ...rest
}: { tone?: NoticeTone } & ComponentPropsWithoutRef<"div">) {
  return (
    <div
      {...rest}
      className={["settings-notice", `settings-notice--${tone}`, className].filter(Boolean).join(" ")}
    />
  );
}

/** Status chip with a tone (`.chip` + tone modifier). */
export function StatusChip({
  tone = "neutral",
  className,
  ...rest
}: { tone?: NoticeTone } & ComponentPropsWithoutRef<"span">) {
  return (
    <span
      {...rest}
      className={["chip", tone !== "neutral" && `settings-chip--${tone}`, className].filter(Boolean).join(" ")}
    />
  );
}
