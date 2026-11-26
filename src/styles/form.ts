import type { CSSProperties } from "react";

/**
 * Form styling for settings/dialog forms.
 *
 * Preferred: the shared classes (src/styles/index.css, Primitives section) —
 *   <div className={formClass.field}>
 *     <label className={formClass.label}>…</label>
 *     <input className={formClass.input} />        // or <Input/> from ui/Input
 *   </div>
 * Classes get hover/focus/disabled/invalid states; inline styles cannot.
 *
 * The inline-style objects below are kept for existing call sites and mirror
 * the same spec (height --control-md, radius-md, --color-control-bg, border
 * token) so those forms already look like `.input`.
 */
export const formClass = {
  field: "field",
  label: "field__label",
  hint: "field__hint",
  error: "field__error",
  input: "input",
  select: "select",
  textarea: "textarea",
  checkbox: "checkbox",
  radio: "radio",
  choice: "choice",
} as const;

export const inputStyle: CSSProperties = {
  width: "100%",
  minHeight: "var(--control-md)",
  padding: "6px var(--space-2)",
  borderRadius: "var(--radius-md)",
  border: "1px solid var(--color-border)",
  backgroundColor: "var(--color-control-bg)",
  color: "var(--color-text-primary)",
  fontSize: "var(--font-base)",
  lineHeight: "18px",
  boxSizing: "border-box",
};

export const labelStyle: CSSProperties = {
  display: "block",
  fontSize: "var(--font-sm)",
  fontWeight: "var(--weight-medium)" as CSSProperties["fontWeight"],
  color: "var(--color-text-secondary)",
  marginBottom: "var(--space-1)",
};

export const fieldGroupStyle: CSSProperties = {
  marginBottom: "var(--space-3)",
};
