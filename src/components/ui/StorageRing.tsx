/**
 * Storage usage ring.
 *
 * Pure SVG so it stays crisp at 100 / 125 / 150% Windows scaling, and so the "free space" arc can
 * be drawn as a real arc rather than a CSS conic-gradient approximation.
 */

import { categoryColor } from '@/lib/categories';

export interface StorageRingProps {
  usedBytes: number;
  totalBytes: number;
  /** Optional category breakdown drawn as nested arcs inside the used portion. */
  segments?: Array<{ category: string; bytes: number }>;
  size?: number;
  thickness?: number;
  label?: string;
  sublabel?: string;
  className?: string;
}

export function StorageRing({
  usedBytes,
  totalBytes,
  segments,
  size = 76,
  thickness = 8,
  label,
  sublabel,
  className,
}: StorageRingProps): React.JSX.Element {
  const total = Math.max(0, totalBytes);
  const used = Math.max(0, Math.min(total, usedBytes));
  const free = Math.max(0, total - used);
  const usedFraction = total > 0 ? used / total : 0;

  const radius = (size - thickness) / 2;
  const center = size / 2;
  const circumference = 2 * Math.PI * radius;

  // Default arc colour escalates as the drive fills up.
  const usedColor = usedFraction > 0.9 ? '#b4462f' : usedFraction > 0.75 ? '#a97a20' : '#24211d';

  const breakdown = (segments ?? [])
    .map((segment) => ({ ...segment, fraction: total > 0 ? segment.bytes / total : 0 }))
    .filter((segment) => segment.fraction > 0)
    .sort((a, b) => b.fraction - a.fraction);

  let offset = 0;
  const innerArcs = breakdown.map((segment) => {
    const length = Math.max(0, segment.fraction * circumference);
    const arc = (
      <circle
        key={segment.category}
        cx={center}
        cy={center}
        r={radius - thickness / 2 - 1}
        fill="none"
        stroke={categoryColor(segment.category as never)}
        strokeWidth={thickness - 3}
        strokeDasharray={`${length} ${circumference - length}`}
        strokeDashoffset={-offset}
        strokeLinecap="butt"
        transform={`rotate(-90 ${center} ${center})`}
      />
    );
    offset += length;
    return arc;
  });

  return (
    <div className={className} style={{ width: size, height: size, position: 'relative' }}>
      <svg width={size} height={size} viewBox={`0 0 ${size} ${size}`} role="img" aria-label={label ?? 'Storage usage'}>
        <circle
          cx={center}
          cy={center}
          r={radius}
          fill="none"
          stroke="var(--color-surface-sunken)"
          strokeWidth={thickness}
        />
        <circle
          cx={center}
          cy={center}
          r={radius}
          fill="none"
          stroke={usedColor}
          strokeWidth={thickness}
          strokeDasharray={`${usedFraction * circumference} ${circumference}`}
          strokeLinecap="round"
          transform={`rotate(-90 ${center} ${center})`}
          style={{ transition: 'stroke-dasharray 240ms ease' }}
        />
        {breakdown.length > 0 ? innerArcs : null}
      </svg>
      <div className="absolute inset-0 flex flex-col items-center justify-center leading-none">
        <span className="dw-tnum text-[13px] font-semibold text-ink">
          {Math.round(usedFraction * 100)}%
        </span>
        {sublabel ? <span className="mt-0.5 text-[9px] text-ink-faint">{sublabel}</span> : null}
      </div>
      {free > 0 ? <span className="sr-only">{`${free} bytes free`}</span> : null}
    </div>
  );
}
