import { Loader } from "lucide-react";
import { useDelayedVisible } from "@/hooks/useDelayedVisible";

interface SpinnerProps {
  size?: number;
  label?: string;
  /** Hold the spinner back for this long so fast loads never flash it (ms). */
  delayMs?: number;
}

/** A centered spinner with optional label text. */
export default function Spinner({ size = 20, label, delayMs = 0 }: SpinnerProps) {
  const visible = useDelayedVisible(delayMs);
  if (!visible) return <div role="status" aria-busy="true" style={{ height: "100%" }} />;
  return (
    <div
      role="status"
      aria-live="polite"
      className="fade-in"
      style={{
        display: "flex",
        flexDirection: "column",
        alignItems: "center",
        justifyContent: "center",
        gap: "10px",
        padding: "40px",
        color: "var(--color-text-secondary)",
        fontSize: "var(--font-base)",
      }}
    >
      <Loader size={size} className="spinner" />
      {label && <span>{label}</span>}
    </div>
  );
}
