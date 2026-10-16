/**
 * Interactive sunburst visualization.
 *
 * Rendering strategy
 * ------------------
 * - Angles come from `buildSunburstLayout`, a pure function that is unit-tested and benchmarked
 *   separately. The component only draws.
 * - One `<path>` per arc. `Arc` is memoized and receives only primitives, so moving the mouse
 *   re-renders at most two arcs instead of the whole chart.
 * - `maxVisibleArcs` bounds the DOM: with the backend's default pruning (100 children per folder)
 *   a full-drive scan renders a few thousand paths at most, and the angular threshold drops the
 *   sub-pixel ones.
 * - Hover and selection are kept in React state but never in the layout memo, so panning a huge
 *   tree never recomputes the geometry.
 */

import { memo, useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import { ArrowUp, CornerLeftUp } from 'lucide-react';
import type { StorageCategory, TreeNode } from '@/types/contracts';
import { buildSunburstLayout, sunburstArcPath, type SunburstArc } from './layout';
import { categoryColor, shadeCategoryColor } from '@/lib/categories';
import { formatBytes, formatCount } from '@/lib/format';
import { FloatingTooltip } from '@/components/ui/feedback';
import { FileIcon } from '@/lib/icons';

export interface SunburstProps {
  /** The node to render as the centre of the chart. */
  root: TreeNode;
  selectedPath: string | null;
  onSelect: (path: string | null) => void;
  /** Double-click / Enter / the centre button. */
  onNavigate: (path: string) => void;
  /** Click the centre to go back up one level. */
  onNavigateUp: () => void;
  canNavigateUp: boolean;
  onContextMenu: (path: string, x: number, y: number) => void;
  maxVisibleArcs?: number;
  className?: string;
}

const CENTER_RATIO = 0.16;

export function Sunburst({
  root,
  selectedPath,
  onSelect,
  onNavigate,
  onNavigateUp,
  canNavigateUp,
  onContextMenu,
  maxVisibleArcs = 2600,
  className,
}: SunburstProps): React.JSX.Element {
  const containerRef = useRef<HTMLDivElement>(null);
  const [size, setSize] = useState({ width: 720, height: 560 });
  const [hovered, setHovered] = useState<SunburstArc | null>(null);
  const [pointer, setPointer] = useState({ x: 0, y: 0 });
  const [keyboardIndex, setKeyboardIndex] = useState<number>(-1);

  /* ------------------------------- sizing ------------------------------- */
  useLayoutEffect(() => {
    const element = containerRef.current;
    if (!element) return;
    const measure = () => {
      const rect = element.getBoundingClientRect();
      setSize({
        width: Math.max(240, Math.floor(rect.width)),
        height: Math.max(240, Math.floor(rect.height)),
      });
    };
    measure();
    if (typeof ResizeObserver === 'undefined') return;
    const observer = new ResizeObserver(measure);
    observer.observe(element);
    return () => observer.disconnect();
  }, []);

  const layout = useMemo(
    () =>
      buildSunburstLayout(root, {
        maxVisibleArcs,
        radius: 100,
        innerRadiusFraction: CENTER_RATIO,
      }),
    [root, maxVisibleArcs],
  );

  const radius = Math.max(60, Math.min(size.width, size.height) / 2 - 12);
  const center = { x: size.width / 2, y: size.height / 2 };
  const selected = useMemo(
    () => layout.arcs.find((arc) => arc.path === selectedPath) ?? null,
    [layout.arcs, selectedPath],
  );

  /* ---------------------------- interactions ---------------------------- */
  const handleMove = useCallback((event: React.MouseEvent<SVGPathElement>, arc: SunburstArc) => {
    const rect = (event.currentTarget.ownerSVGElement as SVGSVGElement).getBoundingClientRect();
    setHovered(arc);
    setPointer({ x: event.clientX - rect.left, y: event.clientY - rect.top });
  }, []);

  const handleClick = useCallback(
    (arc: SunburstArc) => {
      onSelect(arc.path);
      if (arc.kind === 'folder') {
        // Single click selects; a second click on the same folder opens it.
        if (selectedPath === arc.path) onNavigate(arc.path);
      }
    },
    [onNavigate, onSelect, selectedPath],
  );

  const handleDoubleClick = useCallback(
    (arc: SunburstArc) => {
      if (arc.kind === 'folder') onNavigate(arc.path);
      else onSelect(arc.path);
    },
    [onNavigate, onSelect],
  );

  /* --------------------------- keyboard access --------------------------- */
  const onKeyDown = useCallback(
    (event: React.KeyboardEvent<SVGSVGElement>) => {
      const arcs = layout.arcs;
      if (arcs.length === 0) return;
      const currentIndex = keyboardIndex >= 0 ? keyboardIndex : selected ? arcs.indexOf(selected) : -1;

      if (event.key === 'ArrowRight' || event.key === 'ArrowLeft') {
        event.preventDefault();
        const step = event.key === 'ArrowRight' ? 1 : -1;
        // Walk siblings first, then move outward/inward.
        const from = currentIndex >= 0 ? currentIndex : 0;
        let next = from;
        for (let i = 0; i < arcs.length; i += 1) {
          next = (next + step + arcs.length) % arcs.length;
          if (currentIndex < 0 || arcs[next].depth === arcs[currentIndex].depth) break;
        }
        setKeyboardIndex(next);
        onSelect(arcs[next].path);
        return;
      }
      if (event.key === 'ArrowUp' || event.key === 'ArrowDown') {
        event.preventDefault();
        const step = event.key === 'ArrowDown' ? 1 : -1;
        const from = currentIndex >= 0 ? currentIndex : 0;
        let next = from;
        for (let i = 0; i < arcs.length; i += 1) {
          next = (next + step + arcs.length) % arcs.length;
          if (currentIndex < 0 || arcs[next].depth !== arcs[currentIndex].depth) break;
        }
        setKeyboardIndex(next);
        onSelect(arcs[next].path);
        return;
      }
      if (event.key === 'Enter' || event.key === ' ') {
        event.preventDefault();
        const arc = currentIndex >= 0 ? arcs[currentIndex] : selected;
        if (arc) {
          onSelect(arc.path);
          if (arc.kind === 'folder') onNavigate(arc.path);
        }
        return;
      }
      if (event.key === 'Backspace') {
        event.preventDefault();
        if (canNavigateUp) onNavigateUp();
        return;
      }
      if (event.key === 'Escape') {
        onSelect(null);
      }
    },
    [canNavigateUp, keyboardIndex, layout.arcs, onNavigate, onNavigateUp, onSelect, selected],
  );

  useEffect(() => {
    setKeyboardIndex(-1);
  }, [root.path]);

  if (layout.empty) {
    return (
      <div ref={containerRef} className={className} data-testid="sunburst-empty">
        <EmptySunburst size={size} message="This folder is empty — there is nothing to draw." />
      </div>
    );
  }

  const scale = radius / 100;
  const centerRadius = 100 * CENTER_RATIO * scale;

  return (
    <div ref={containerRef} className={className} data-testid="sunburst">
      <svg
        width={size.width}
        height={size.height}
        viewBox={`0 0 ${size.width} ${size.height}`}
        tabIndex={0}
        role="application"
        aria-label={`Sunburst chart of ${root.name}. Use arrow keys to move between segments, Enter to open, Backspace to go up.`}
        onKeyDown={onKeyDown}
        onMouseLeave={() => setHovered(null)}
        className="outline-none"
      >
        <g transform={`translate(${center.x} ${center.y}) scale(${scale})`}>
          {layout.arcs.map((arc) => (
            <Arc
              key={arc.path}
              arc={arc}
              scale={scale}
              selected={selected?.path === arc.path}
              hovered={hovered?.path === arc.path}
              dimmed={hovered !== null && hovered.path !== arc.path}
              onMove={handleMove}
              onLeave={() => setHovered(null)}
              onClick={handleClick}
              onDoubleClick={handleDoubleClick}
              onContextMenu={(event) => {
                event.preventDefault();
                onContextMenu(arc.path, event.clientX, event.clientY);
              }}
            />
          ))}

          {/* centre: the current root */}
          <g
            className="cursor-pointer"
            onClick={() => (canNavigateUp ? onNavigateUp() : onSelect(null))}
            role="button"
            aria-label={canNavigateUp ? `Go up from ${root.name}` : `Current root ${root.name}`}
          >
            <circle r={centerRadius} fill="var(--color-surface)" stroke="var(--color-line)" />
            <circle r={centerRadius - 4} fill="var(--color-canvas)" />
          </g>
        </g>

        {/* centre labels live outside the scaled group so text never distorts */}
        <text
          x={center.x}
          y={center.y - 4}
          textAnchor="middle"
          className="fill-[var(--color-ink)]"
          style={{ fontSize: 11, fontWeight: 600 }}
        >
          {truncate(root.name, 18)}
        </text>
        <text
          x={center.x}
          y={center.y + 10}
          textAnchor="middle"
          className="dw-tnum fill-[var(--color-ink-muted)]"
          style={{ fontSize: 10 }}
        >
          {formatBytes(root.sizeBytes)}
        </text>
        {canNavigateUp ? (
          <g
            transform={`translate(${center.x} ${center.y + centerRadius * 0.55})`}
            className="cursor-pointer"
            onClick={onNavigateUp}
            role="button"
            aria-label="Go up one level"
          >
            <circle r={9} fill="var(--color-surface-sunken)" />
            <ArrowUp x={-5} y={-5} size={10} className="text-[var(--color-ink-muted)]" />
          </g>
        ) : null}
      </svg>

      {layout.collapsedArcs > 0 ? (
        <p className="pointer-events-none absolute bottom-1 left-1/2 -translate-x-1/2 rounded-full border border-line bg-surface/90 px-2 py-0.5 text-[10px] text-ink-faint">
          {formatCount(layout.collapsedArcs)} items too small to draw ·{' '}
          {formatBytes(layout.collapsedBytes)}
        </p>
      ) : null}

      {hovered ? (
        <FloatingTooltip anchor={pointer} container={size} visible>
          <SunburstTooltip arc={hovered} root={root} />
        </FloatingTooltip>
      ) : null}
    </div>
  );
}

/* ------------------------------------------------------------------ *
 * Arc
 * ------------------------------------------------------------------ */

const Arc = memo(function Arc({
  arc,
  scale,
  selected,
  hovered,
  dimmed,
  onMove,
  onLeave,
  onClick,
  onDoubleClick,
  onContextMenu,
}: {
  arc: SunburstArc;
  scale: number;
  selected: boolean;
  hovered: boolean;
  dimmed: boolean;
  onMove: (event: React.MouseEvent<SVGPathElement>, arc: SunburstArc) => void;
  onLeave: () => void;
  onClick: (arc: SunburstArc) => void;
  onDoubleClick: (arc: SunburstArc) => void;
  onContextMenu: (event: React.MouseEvent<SVGPathElement>) => void;
}): React.JSX.Element {
  // Depth shading keeps sibling rings distinguishable without extra hues.
  const shade = arc.depth <= 2 ? 0 : (arc.depth - 2) * 0.06;
  const fill = arc.isSynthetic
    ? 'var(--color-line-strong)'
    : shadeCategoryColor(arc.category as StorageCategory, Math.min(0.35, shade));

  return (
    <path
      d={sunburstArcPath(arc, 0.18)}
      fill={fill}
      fillOpacity={dimmed ? 0.55 : 1}
      stroke={selected ? 'var(--color-accent)' : hovered ? 'var(--color-ink-strong)' : 'transparent'}
      strokeWidth={selected ? 1.6 / scale : hovered ? 1.2 / scale : 0}
      className="cursor-pointer transition-[fill-opacity,stroke] duration-75"
      onMouseMove={(event) => onMove(event, arc)}
      onMouseLeave={onLeave}
      onClick={() => onClick(arc)}
      onDoubleClick={() => onDoubleClick(arc)}
      onContextMenu={onContextMenu}
      role="button"
      tabIndex={-1}
      aria-label={`${arc.name}, ${formatBytes(arc.sizeBytes)}, ${arc.kind}`}
      data-testid="sunburst-arc"
      data-path={arc.path}
      data-size={arc.sizeBytes}
    />
  );
});

function SunburstTooltip({ arc, root }: { arc: SunburstArc; root: TreeNode }): React.JSX.Element {
  const share = root.sizeBytes > 0 ? (arc.sizeBytes / root.sizeBytes) * 100 : 0;
  return (
    <div>
      <div className="flex items-center gap-1.5">
        <FileIcon
          name={arc.name}
          extension={null}
          kind={arc.kind}
          category={arc.category}
          className="size-3.5 shrink-0"
        />
        <p className="truncate text-[12px] font-semibold text-ink" title={arc.name}>
          {arc.name}
        </p>
      </div>
      <p className="dw-tnum mt-1 text-[11px] text-ink">{formatBytes(arc.sizeBytes)}</p>
      <dl className="dw-tnum mt-0.5 space-y-px text-[10px] text-ink-muted">
        <div className="flex justify-between gap-3">
          <dt>Share</dt>
          <dd>{share.toFixed(share < 1 ? 2 : 1)}%</dd>
        </div>
        <div className="flex justify-between gap-3">
          <dt>Depth</dt>
          <dd>level {arc.depth}</dd>
        </div>
      </dl>
      <p className="mt-1 break-all font-mono text-[9px] leading-relaxed text-ink-faint" title={arc.path}>
        {arc.path}
      </p>
      {arc.kind === 'folder' ? (
        <p className="mt-1 flex items-center gap-1 text-[9px] text-ink-faint">
          <CornerLeftUp aria-hidden className="size-2.5" />
          Double-click to open
        </p>
      ) : null}
    </div>
  );
}

function EmptySunburst({ size, message }: { size: { width: number; height: number }; message: string }): React.JSX.Element {
  const radius = Math.max(60, Math.min(size.width, size.height) / 2 - 12);
  return (
    <svg width={size.width} height={size.height} viewBox={`0 0 ${size.width} ${size.height}`} role="img" aria-label={message}>
      <g transform={`translate(${size.width / 2} ${size.height / 2})`}>
        <circle r={radius} fill="none" stroke="var(--color-line)" strokeWidth={1} />
        <circle r={radius * 0.66} fill="none" stroke="var(--color-line)" strokeWidth={1} strokeDasharray="3 4" />
        <circle r={radius * 0.33} fill="var(--color-surface)" stroke="var(--color-line)" />
      </g>
      <text
        x={size.width / 2}
        y={size.height / 2 + radius + 18}
        textAnchor="middle"
        className="fill-[var(--color-ink-faint)]"
        style={{ fontSize: 11 }}
      >
        {message}
      </text>
    </svg>
  );
}

function truncate(text: string, max: number): string {
  return text.length <= max ? text : `${text.slice(0, max - 1)}…`;
}

export { categoryColor };
