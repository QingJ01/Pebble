/**
 * Button — the shared text button (styles: `.btn*` in src/styles/index.css).
 *
 *   <Button variant="primary" size="md" onClick={save}>Save</Button>
 *   <Button variant="danger" loading={busy}>Delete</Button>          // confirm dialog
 *   <Button variant="danger-ghost" size="sm">Remove</Button>          // inline destructive
 *   <Button variant="link" size="sm">Learn more</Button>
 *   <Button variant="secondary" icon={<Archive size={14} />}>Archive</Button>
 *
 * - variant: "primary" | "secondary" (default) | "ghost" | "danger" |
 *   "danger-ghost" | "link". At most ONE primary per view. "danger" (solid red)
 *   only confirms a destructive action; inline destructive triggers use
 *   "danger-ghost". "link" is accent text, underlined on hover.
 * - size: "sm" (28px) | "md" (32px, default) | "lg" (36px)
 * - pressed: toggle state → aria-pressed (neutral selected fill). Omit otherwise.
 * - Native elements that must look like a button (e.g. <a>, <label>):
 *   className={buttonClassName("secondary", "sm")}
 * - loading: shows a spinner, sets aria-busy + aria-disabled and swallows clicks,
 *   but keeps the button focusable (focus is not lost while an action runs).
 * - disabled: native disabled. Prefer `loading` for in-flight actions.
 * - type defaults to "button"; `ref` is forwarded (React 19 ref-as-prop).
 * Hover/active/disabled/focus-visible are pure CSS — never add JS hover handlers.
 */
import type { ComponentPropsWithRef, MouseEvent, ReactNode } from "react";
import { Loader2 } from "lucide-react";

export type ButtonVariant = "primary" | "secondary" | "ghost" | "danger" | "danger-ghost" | "link";
export type ButtonSize = "sm" | "md" | "lg";

export interface ButtonProps extends Omit<ComponentPropsWithRef<"button">, "aria-pressed"> {
  variant?: ButtonVariant;
  size?: ButtonSize;
  loading?: boolean;
  /** Toggle state → aria-pressed. Omit for non-toggle buttons. */
  pressed?: boolean;
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
  pressed,
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
      aria-pressed={pressed === undefined ? undefined : pressed}
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
