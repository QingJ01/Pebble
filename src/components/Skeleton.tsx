import { useDelayedVisible } from "@/hooks/useDelayedVisible";

interface SkeletonProps {
  width?: string;
  height?: string;
  borderRadius?: string;
  style?: React.CSSProperties;
}

/** A single shimmer placeholder bar. */
export function Skeleton({
  width = "100%",
  height = "14px",
  borderRadius = "4px",
  style,
}: SkeletonProps) {
  return (
    <div
      className="skeleton"
      style={{ width, height, borderRadius, ...style }}
    />
  );
}

/** Skeleton that mimics a mail list item (avatar + 2 text lines). */
export function MessageSkeleton() {
  return (
    <div
      style={{
        display: "flex",
        alignItems: "flex-start",
        gap: "12px",
        padding: "12px 14px",
        borderBottom: "1px solid var(--color-border)",
      }}
    >
      <Skeleton width="32px" height="32px" borderRadius="50%" />
      <div style={{ flex: 1, display: "flex", flexDirection: "column", gap: "8px" }}>
        <Skeleton width="40%" height="12px" />
        <Skeleton width="70%" height="11px" />
        <Skeleton width="90%" height="10px" />
      </div>
    </div>
  );
}

/** Multiple message skeletons stacked. */
export function MessageListSkeleton({ count = 6, delayMs = 0 }: { count?: number; delayMs?: number }) {
  const visible = useDelayedVisible(delayMs);
  if (!visible) return <div role="status" aria-busy="true" />;
  return (
    <div role="status" aria-live="polite" className="fade-in">
      {Array.from({ length: count }, (_, i) => (
        <MessageSkeleton key={i} />
      ))}
    </div>
  );
}

/** Skeleton for the kanban board columns. */
export function KanbanSkeleton() {
  return (
    <div role="status" aria-live="polite" className="fade-in" style={{ display: "flex", gap: "16px", padding: "20px" }}>
      {[1, 2, 3].map((col) => (
        <div
          key={col}
          style={{
            flex: 1,
            display: "flex",
            flexDirection: "column",
            gap: "10px",
          }}
        >
          <Skeleton width="60%" height="16px" />
          <Skeleton height="80px" borderRadius="8px" />
          <Skeleton height="80px" borderRadius="8px" />
        </div>
      ))}
    </div>
  );
}

/** Skeleton for message detail view. */
export function MessageDetailSkeleton() {
  return (
    <div role="status" aria-live="polite" className="fade-in" style={{ padding: "24px", display: "flex", flexDirection: "column", gap: "16px" }}>
      <Skeleton width="55%" height="18px" />
      <div style={{ display: "flex", gap: "10px", alignItems: "center" }}>
        <Skeleton width="28px" height="28px" borderRadius="50%" />
        <div style={{ display: "flex", flexDirection: "column", gap: "6px" }}>
          <Skeleton width="140px" height="12px" />
          <Skeleton width="200px" height="10px" />
        </div>
      </div>
      <div style={{ height: "1px", background: "var(--color-border)" }} />
      <Skeleton width="100%" height="12px" />
      <Skeleton width="95%" height="12px" />
      <Skeleton width="80%" height="12px" />
      <Skeleton width="100%" height="12px" />
      <Skeleton width="60%" height="12px" />
    </div>
  );
}
