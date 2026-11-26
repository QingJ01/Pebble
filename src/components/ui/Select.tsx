/**
 * Select — native <select> with the shared control look (styles: `.select`
 * in src/styles/index.css): appearance none, custom chevron, control heights.
 *
 *   <Select value={mode} onChange={(e) => setMode(e.target.value)}>
 *     <option value="a">A</option>
 *   </Select>
 *
 * - size: "sm" (28) | "md" (32, default) | "lg" (36).
 * - Native elements elsewhere can use className={selectClassName()}.
 */
import type { ComponentPropsWithRef } from "react";
import type { ControlSize } from "./Input";

export function selectClassName(size: ControlSize = "md", extra?: string) {
  return ["select", size !== "md" && `input--${size}`, extra].filter(Boolean).join(" ");
}

export interface SelectProps extends Omit<ComponentPropsWithRef<"select">, "size"> {
  size?: ControlSize;
  invalid?: boolean;
}

export default function Select({ size = "md", invalid, className, children, ...rest }: SelectProps) {
  return (
    <select
      {...rest}
      aria-invalid={invalid || rest["aria-invalid"] || undefined}
      className={selectClassName(size, className)}
    >
      {children}
    </select>
  );
}
