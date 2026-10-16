/**
 * Windowed list.
 *
 * The file list can hold hundreds of thousands of rows. Rendering them all would lock the UI, so
 * only the visible slice (plus a small overscan) is mounted. Row height is fixed, which is what
 * makes the offset arithmetic exact.
 */

import { useCallback, useEffect, useRef, useState, type ReactNode } from 'react';

export interface VirtualListProps<T> {
  items: T[];
  rowHeight: number;
  height: number;
  renderRow: (item: T, index: number) => ReactNode;
  keyOf: (item: T, index: number) => string;
  overscan?: number;
  className?: string;
  onScrollBottom?: () => void;
  /** Rendered when there is nothing to show. */
  empty?: ReactNode;
  ariaLabel?: string;
}

export function VirtualList<T>({
  items,
  rowHeight,
  height,
  renderRow,
  keyOf,
  overscan = 8,
  className,
  onScrollBottom,
  empty,
  ariaLabel,
}: VirtualListProps<T>): React.JSX.Element {
  const [scrollTop, setScrollTop] = useState(0);
  const containerRef = useRef<HTMLDivElement>(null);

  const onScroll = useCallback(() => {
    const element = containerRef.current;
    if (!element) return;
    setScrollTop(element.scrollTop);
    if (onScrollBottom && element.scrollHeight - element.scrollTop - element.clientHeight < rowHeight * 2) {
      onScrollBottom();
    }
  }, [onScrollBottom, rowHeight]);

  // Reset the scroll offset when the data set changes identity.
  useEffect(() => {
    if (containerRef.current) containerRef.current.scrollTop = 0;
    setScrollTop(0);
  }, [items]);

  const total = items.length;
  const visibleCount = Math.ceil(height / rowHeight) + overscan * 2;
  const startIndex = Math.max(0, Math.floor(scrollTop / rowHeight) - overscan);
  const endIndex = Math.min(total, startIndex + visibleCount);
  const slice = items.slice(startIndex, endIndex);

  if (total === 0 && empty) {
    return (
      <div ref={containerRef} className={className} style={{ height }} role="list" aria-label={ariaLabel}>
        {empty}
      </div>
    );
  }

  return (
    <div
      ref={containerRef}
      className={className}
      style={{ height, overflowY: 'auto', contain: 'strict' }}
      onScroll={onScroll}
      role="list"
      aria-label={ariaLabel}
      aria-rowcount={total}
    >
      <div style={{ height: total * rowHeight, position: 'relative' }}>
        <div style={{ transform: `translateY(${startIndex * rowHeight}px)` }}>
          {slice.map((item, offset) => (
            <div key={keyOf(item, startIndex + offset)} style={{ height: rowHeight }} role="listitem">
              {renderRow(item, startIndex + offset)}
            </div>
          ))}
        </div>
      </div>
    </div>
  );
}
