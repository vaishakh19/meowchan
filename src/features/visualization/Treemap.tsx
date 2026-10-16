/**
 * Treemap visualization.
 *
 * The rectangular alternative to the sunburst: the same data, laid out with a squarified
 * treemap. Depth is capped and tiny cells are dropped so the chart stays legible; the total area
 * always equals the analyzed bytes.
 */

import { useLayoutEffect, useMemo, useRef, useState } from 'react';
import type { TreeNode } from '@/types/contracts';
import { buildTreemapLayout } from './layout';
import { categoryColor, shadeCategoryColor } from '@/lib/categories';
import { formatBytes } from '@/lib/format';
import { FloatingTooltip } from '@/components/ui/feedback';
import { FileIcon } from '@/lib/icons';

export interface TreemapProps {
  root: TreeNode;
  selectedPath: string | null;
  onSelect: (path: string | null) => void;
  onNavigate: (path: string) => void;
  onContextMenu: (path: string, x: number, y: number) => void;
  className?: string;
}

export function Treemap({
  root,
  selectedPath,
  onSelect,
  onNavigate,
  onContextMenu,
  className,
}: TreemapProps): React.JSX.Element {
  const containerRef = useRef<HTMLDivElement>(null);
  const [size, setSize] = useState({ width: 720, height: 560 });
  const [hovered, setHovered] = useState<{ path: string; x: number; y: number } | null>(null);

  useLayoutEffect(() => {
    const element = containerRef.current;
    if (!element) return;
    const measure = () => {
      const rect = element.getBoundingClientRect();
      setSize({ width: Math.max(240, Math.floor(rect.width)), height: Math.max(240, Math.floor(rect.height)) });
    };
    measure();
    if (typeof ResizeObserver === 'undefined') return;
    const observer = new ResizeObserver(measure);
    observer.observe(element);
    return () => observer.disconnect();
  }, []);

  const layout = useMemo(
    () => buildTreemapLayout(root, { width: size.width, height: size.height, maxDepth: 5, minArea: 320 }),
    [root, size.width, size.height],
  );

  const hoveredCell = hovered ? layout.cells.find((cell) => cell.path === hovered.path) ?? null : null;

  return (
    <div ref={containerRef} className={className} data-testid="treemap">
      <svg
        width={size.width}
        height={size.height}
        viewBox={`0 0 ${size.width} ${size.height}`}
        role="img"
        aria-label={`Treemap of ${root.name}, ${formatBytes(root.sizeBytes)}`}
        onMouseLeave={() => setHovered(null)}
      >
        {layout.cells.map((cell) => {
          const selected = cell.path === selectedPath;
          const shade = cell.depth <= 1 ? 0 : (cell.depth - 1) * 0.07;
          return (
            <g
              key={cell.path}
              transform={`translate(${cell.x} ${cell.y})`}
              className="cursor-pointer"
              onClick={() => {
                onSelect(cell.path);
                if (selected && cell.depth > 0) onNavigate(cell.path);
              }}
              onDoubleClick={() => cell.depth > 0 && onNavigate(cell.path)}
              onMouseMove={(event) => {
                const rect = (event.currentTarget.ownerSVGElement as SVGSVGElement).getBoundingClientRect();
                setHovered({ path: cell.path, x: event.clientX - rect.left, y: event.clientY - rect.top });
              }}
              onContextMenu={(event) => {
                event.preventDefault();
                onContextMenu(cell.path, event.clientX, event.clientY);
              }}
              role="button"
              tabIndex={-1}
              aria-label={`${cell.name}, ${formatBytes(cell.sizeBytes)}`}
              data-testid="treemap-cell"
            >
              <rect
                width={Math.max(0, cell.width)}
                height={Math.max(0, cell.height)}
                rx={3}
                fill={shadeCategoryColor(cell.category, Math.min(0.4, shade))}
                fillOpacity={hovered && hovered.path !== cell.path ? 0.6 : 1}
                stroke={selected ? 'var(--color-accent)' : 'rgba(36,33,29,0.08)'}
                strokeWidth={selected ? 2 : 1}
              />
              {cell.width > 56 && cell.height > 22 ? (
                <text
                  x={5}
                  y={14}
                  className="pointer-events-none fill-[var(--color-ink)]"
                  style={{ fontSize: 10, fontWeight: 500 }}
                >
                  {truncate(cell.name, Math.floor(cell.width / 6))}
                </text>
              ) : null}
              {cell.width > 56 && cell.height > 36 ? (
                <text
                  x={5}
                  y={26}
                  className="dw-tnum pointer-events-none fill-[var(--color-ink-muted)]"
                  style={{ fontSize: 9 }}
                >
                  {formatBytes(cell.sizeBytes)}
                </text>
              ) : null}
            </g>
          );
        })}
      </svg>

      {hoveredCell && hovered ? (
        <FloatingTooltip anchor={{ x: hovered.x, y: hovered.y }} container={size} visible>
          <div className="flex items-center gap-1.5">
            <FileIcon
              name={hoveredCell.name}
              extension={null}
              kind={hoveredCell.depth > 0 ? 'folder' : 'folder'}
              category={hoveredCell.category}
              className="size-3.5 shrink-0"
            />
            <p className="truncate text-[12px] font-semibold text-ink">{hoveredCell.name}</p>
          </div>
          <p className="dw-tnum mt-1 text-[11px] text-ink">{formatBytes(hoveredCell.sizeBytes)}</p>
          <p className="mt-0.5 break-all font-mono text-[9px] text-ink-faint">{hoveredCell.path}</p>
        </FloatingTooltip>
      ) : null}

      {layout.empty ? (
        <p className="absolute inset-0 flex items-center justify-center text-xs text-ink-muted">
          This folder is empty — there is nothing to draw.
        </p>
      ) : null}
    </div>
  );
}

function truncate(text: string, max: number): string {
  if (max <= 1) return '';
  return text.length <= max ? text : `${text.slice(0, max - 1)}…`;
}

export { categoryColor };
