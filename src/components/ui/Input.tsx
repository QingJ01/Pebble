/**
 * Input / Textarea — the shared text controls (styles: `.input`, `.textarea`
 * in src/styles/index.css, Primitives section).
 *
 *   <Input value={v} onChange={…} placeholder="you@example.com" />
 *   <Input size="lg" type="search" />            // 36px (main search)
 *   <Textarea rows={4} value={notes} onChange={…} />
 *
 * - size: "sm" (28) | "md" (32, default) | "lg" (36) — same scale as Button.
 * - invalid: sets aria-invalid (red border). Pair with a .field__error message.
 * - Focus style is CSS only (border + --shadow-focus); never add JS focus styling.
 * - For inline-style call sites that can't take a class yet, see src/styles/form.ts.
 */
import type { ComponentPropsWithRef } from "react";

export type ControlSize = "sm" | "md" | "lg";

export function inputClassName(size: ControlSize = "md", extra?: string) {
  return ["input", size !== "md" && `input--${size}`, extra].filter(Boolean).join(" ");
}

export interface InputProps extends Omit<ComponentPropsWithRef<"input">, "size"> {
  size?: ControlSize;
  invalid?: boolean;
}

export default function Input({ size = "md", invalid, className, ...rest }: InputProps) {
  return (
    <input
      {...rest}
      aria-invalid={invalid || rest["aria-invalid"] || undefined}
      className={inputClassName(size, className)}
    />
  );
}

export interface TextareaProps extends ComponentPropsWithRef<"textarea"> {
  invalid?: boolean;
}

export function Textarea({ invalid, className, ...rest }: TextareaProps) {
  return (
    <textarea
      {...rest}
      aria-invalid={invalid || rest["aria-invalid"] || undefined}
      className={["textarea", className].filter(Boolean).join(" ")}
    />
  );
}
