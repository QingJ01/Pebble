/**
 * IconButton — icon-only button with a guaranteed accessible name and hit area
 * (styles: `.icon-btn*` in src/styles/index.css).
 *
 *   <IconButton label={t("common.close")} icon={<X size={14} />} onClick={close} />
 *   <IconButton label={t("messageActions.star")} icon={<Star size={14} />} pressed={starred} />
 *
 * - label (required): becomes aria-label and, unless `tooltip` overrides it,
 *   the native title tooltip. Pass tooltip={false} to suppress the tooltip.
 * - size: "sm" (28×28, default) | "md" (32×32) | "lg" (36×36). Use one size per
 *   toolbar row, matching any text Buttons in it. Hit area never drops below 28px.
 * - variant: "ghost" (default) | "secondary" | "danger" (red icon, ghost)
 * - pressed: toggle state → aria-pressed (neutral selected fill). Omit for non-toggles.
 * - iconButtonClassName(variant, size) styles a native element the same way.
 * - loading: spinner replaces the icon; aria-busy + aria-disabled, clicks ignored.
 */
import type { ComponentPropsWithRef, MouseEvent, ReactNode } from "react";
import { Loader2 } from "lucide-react";

export type IconButtonVariant = "ghost" | "secondary" | "danger";
export type IconButtonSize = "sm" | "md" | "lg";

export function iconButtonClassName(variant: IconButtonVariant = "ghost", size: IconButtonSize = "sm", extra?: string) {
  return ["icon-btn", `icon-btn--${variant}`, `icon-btn--${size}`, extra].filter(Boolean).join(" ");
}

export interface IconButtonProps
  extends Omit<ComponentPropsWithRef<"button">, "children" | "aria-label" | "title" | "aria-pressed"> {
  label: string;
  icon: ReactNode;
  tooltip?: string | false;
  size?: IconButtonSize;
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
      className={iconButtonClassName(variant, size, className)}
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
