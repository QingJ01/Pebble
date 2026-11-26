import { useDelayedVisible } from "@/hooks/useDelayedVisible";
import "./mail-list.css";
import "./message-reader.css";
import "../features/kanban/kanban.css";

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
  borderRadius = "var(--radius-sm)",
  style,
}: SkeletonProps) {
  return (
    <div
      className="skeleton"
      style={{ width, height, borderRadius, ...style }}
    />
  );
}

/** Skeleton that mimics a mail list row (sender + date, subject, snippet; 72px). */
export function MessageSkeleton() {
  return (
    <div className="message-list-row skeleton-row" aria-hidden="true">
      <div className="skeleton-row__line">
        <Skeleton width="40%" height="12px" />
        <Skeleton width="36px" height="10px" />
      </div>
      <Skeleton width="70%" height="12px" />
      <Skeleton width="90%" height="10px" />
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

/** Skeleton for the kanban board columns (same column wells as the board). */
export function KanbanSkeleton() {
  return (
    <div role="status" aria-live="polite" className="fade-in kanban-board">
      {[1, 2, 3].map((col) => (
        <div key={col} className="kanban-column">
          <div className="kanban-column__header">
            <Skeleton width="40%" height="14px" />
          </div>
          <div className="kanban-column__cards">
            <Skeleton height="72px" borderRadius="var(--radius-lg)" />
            <Skeleton height="72px" borderRadius="var(--radius-lg)" />
          </div>
        </div>
      ))}
    </div>
  );
}

/** Skeleton for the reading pane: toolbar row, subject + sender card, body. */
export function MessageDetailSkeleton() {
  return (
    <div role="status" aria-live="polite" className="fade-in message-detail-container">
      <div className="reader-toolbar">
        <Skeleton width="160px" height="16px" />
      </div>
      <div className="reader-header">
        <Skeleton width="55%" height="18px" />
        <Skeleton height="56px" borderRadius="var(--radius-lg)" />
      </div>
      <div className="reader-body skeleton-stack">
        <Skeleton width="100%" height="12px" />
        <Skeleton width="95%" height="12px" />
        <Skeleton width="80%" height="12px" />
        <Skeleton width="100%" height="12px" />
        <Skeleton width="60%" height="12px" />
      </div>
    </div>
  );
}
