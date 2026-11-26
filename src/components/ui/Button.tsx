/**
 * Button — the shared text button (styles: `.btn*` in src/styles/index.css).
 *
 *   <Button variant="primary" size="md" onClick={save}>Save</Button>
 *   <Button variant="danger" loading={busy}>Delete</Button>
 *   <Button variant="secondary" icon={<Archive size={14} />}>Archive</Button>
 *
 * - variant: "primary" | "secondary" (default) | "ghost" | "danger"
 * - size: "sm" (28px) | "md" (32px, default)
 * - loading: shows a spinner, sets aria-busy + aria-disabled and swallows clicks,
 *   but keeps the button focusable (focus is not lost while an action runs).
 * - disabled: native disabled. Prefer `loading` for in-flight actions.
 * - type defaults to "button"; `ref` is forwarded (React 19 ref-as-prop).
 * Hover/active/disabled/focus-visible are pure CSS — never add JS hover handlers.
 */
import type { ComponentPropsWithRef, MouseEvent, ReactNode } from "react";
import { Loader2 } from "lucide-react";

export type ButtonVariant = "primary" | "secondary" | "ghost" | "danger";
export type ButtonSize = "sm" | "md";

export interface ButtonProps extends ComponentPropsWithRef<"button"> {
  variant?: ButtonVariant;
  size?: ButtonSize;
  loading?: boolean;
  /** Leading icon (rendered before children, hidden from assistive tech). */
  icon?: ReactNode;
}

export function buttonClassName(variant: ButtonVariant = "secondary", size: ButtonSize = "md", extra?: string) {
  return ["btn", `btn--${variant}`, `btn--${size}`, extra].filter(Boolean).join(" ");
}

export default function Button({
  variant = "secondary",
  size = "md",
  loading = false,
  icon,
  className,
  type = "button",
  onClick,
  children,
  "aria-disabled": ariaDisabled,
  ...rest
}: ButtonProps) {
  const inert = loading || ariaDisabled === true || ariaDisabled === "true";
  const handleClick = (event: MouseEvent<HTMLButtonElement>) => {
    if (inert) {
      event.preventDefault();
      return;
    }
    onClick?.(event);
  };

  return (
    <button
      {...rest}
      type={type}
      className={buttonClassName(variant, size, className)}
      aria-busy={loading || undefined}
      aria-disabled={inert || undefined}
      onClick={handleClick}
    >
      {loading ? (
        <Loader2 className="btn__spinner" size={size === "sm" ? 12 : 14} aria-hidden="true" />
      ) : icon ? (
        <span aria-hidden="true" style={{ display: "inline-flex" }}>{icon}</span>
      ) : null}
      {children}
    </button>
  );
}
