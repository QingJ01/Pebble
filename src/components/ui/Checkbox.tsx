/**
 * Checkbox / Radio — custom-drawn, neutral when checked (styles: `.checkbox`,
 * `.radio`, `.choice` in src/styles/index.css). Orange is NOT used for checked
 * state; it stays reserved for primary actions, unread state and the FAB.
 *
 *   <Checkbox checked={on} onChange={(e) => setOn(e.target.checked)} label="Show images" />
 *   <Radio name="mode" value="a" checked={mode === "a"} onChange={…} label="Realtime" />
 *   <Checkbox aria-label="Select all" indeterminate={some && !all} … />   // no visible label
 *
 * - With `label`, renders <label class="choice"> wrapping the input + text.
 * - indeterminate: sets the DOM property (no HTML attribute exists).
 */
import { useEffect, useRef, type ComponentPropsWithRef, type ReactNode } from "react";

interface ChoiceProps extends Omit<ComponentPropsWithRef<"input">, "type"> {
  label?: ReactNode;
}

export interface CheckboxProps extends ChoiceProps {
  indeterminate?: boolean;
}

function wrap(label: ReactNode | undefined, input: ReactNode, disabled?: boolean) {
  if (label === undefined || label === null) return input;
  return (
    <label className="choice" style={disabled ? { cursor: "not-allowed" } : undefined}>
      {input}
      <span>{label}</span>
    </label>
  );
}

export default function Checkbox({ label, indeterminate, className, ref, ...rest }: CheckboxProps) {
  const innerRef = useRef<HTMLInputElement | null>(null);
  useEffect(() => {
    if (innerRef.current) innerRef.current.indeterminate = Boolean(indeterminate);
  }, [indeterminate]);
  const setRef = (node: HTMLInputElement | null) => {
    innerRef.current = node;
    if (typeof ref === "function") ref(node);
    else if (ref) ref.current = node;
  };
  return wrap(
    label,
    <input {...rest} ref={setRef} type="checkbox" className={["checkbox", className].filter(Boolean).join(" ")} />,
    rest.disabled,
  );
}

export function Radio({ label, className, ...rest }: ChoiceProps) {
  return wrap(
    label,
    <input {...rest} type="radio" className={["radio", className].filter(Boolean).join(" ")} />,
    rest.disabled,
  );
}
