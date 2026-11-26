/**
 * IconButton — icon-only button with a guaranteed accessible name and hit area
 * (styles: `.icon-btn*` in src/styles/index.css).
 *
 *   <IconButton label={t("common.close")} icon={<X size={14} />} onClick={close} />
 *   <IconButton label={t("messageActions.star")} icon={<Star size={14} />} pressed={starred} />
 *
 * - label (required): becomes aria-label and, unless `tooltip` overrides it,
 *   the native title tooltip. Pass tooltip={false} to suppress the tooltip.
 * - size: "sm" (28×28, default) | "md" (32×32). Hit area never drops below 28px.
 * - variant: "ghost" (default) | "secondary" | "danger"
 * - pressed: toggle state → aria-pressed (+ accent styling). Omit for non-toggles.
 * - loading: spinner replaces the icon; aria-busy + aria-disabled, clicks ignored.
 */
import type { ComponentPropsWithRef, MouseEvent, ReactNode } from "react";
import { Loader2 } from "lucide-react";

export type IconButtonVariant = "ghost" | "secondary" | "danger";

export interface IconButtonProps
  extends Omit<ComponentPropsWithRef<"button">, "children" | "aria-label" | "title" | "aria-pressed"> {
  label: string;
  icon: ReactNode;
  tooltip?: string | false;
  size?: "sm" | "md";
  variant?: IconButtonVariant;
  pressed?: boolean;
  loading?: boolean;
}

export default function IconButton({
  label,
  icon,
  tooltip,
  size = "sm",
  variant = "ghost",
  pressed,
  loading = false,
  className,
  type = "button",
  onClick,
  "aria-disabled": ariaDisabled,
  ...rest
}: IconButtonProps) {
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
      className={["icon-btn", `icon-btn--${variant}`, `icon-btn--${size}`, className].filter(Boolean).join(" ")}
      aria-label={label}
      title={tooltip === false ? undefined : tooltip ?? label}
      aria-pressed={pressed === undefined ? undefined : pressed}
      aria-busy={loading || undefined}
      aria-disabled={inert || undefined}
      onClick={handleClick}
    >
      {loading ? (
        <Loader2 className="btn__spinner" size={size === "sm" ? 14 : 16} aria-hidden="true" />
      ) : (
        <span aria-hidden="true" style={{ display: "inline-flex" }}>{icon}</span>
      )}
    </button>
  );
}
