/**
 * Right-click context menu.
 *
 * Only ever offers *safe* actions: reveal, copy path, open, focus in the visualization, add to
 * cleanup review. There is deliberately no "Delete" entry — deletion happens only through the
 * cleanup assistant with an explicit confirmation dialog.
 */

import { useEffect, useRef, useState, type ReactNode } from 'react';
import { cx } from './primitives';

export interface ContextMenuItem {
  label: string;
  onSelect: () => void;
  icon?: ReactNode;
  disabled?: boolean;
  danger?: boolean;
  separatorBefore?: boolean;
}

export function useContextMenu(): {
  open: (x: number, y: number, items: ContextMenuItem[]) => void;
  node: ReactNode;
} {
  const [state, setState] = useState<{ x: number; y: number; items: ContextMenuItem[] } | null>(null);
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!state) return;
    const close = () => setState(null);
    const onKey = (event: KeyboardEvent) => {
      if (event.key === 'Escape') close();
    };
    window.addEventListener('click', close);
    window.addEventListener('blur', close);
    window.addEventListener('resize', close);
    document.addEventListener('keydown', onKey);
    return () => {
      window.removeEventListener('click', close);
      window.removeEventListener('blur', close);
      window.removeEventListener('resize', close);
      document.removeEventListener('keydown', onKey);
    };
  }, [state]);

  const node = state ? (
    <div
      ref={ref}
      role="menu"
      className="fixed z-50 min-w-52 overflow-hidden rounded-control border border-line bg-surface py-1 shadow-pop"
      style={{ left: state.x, top: state.y }}
      onClick={(event) => event.stopPropagation()}
    >
      {state.items.map((item, index) => (
        <div key={item.label}>
          {item.separatorBefore && index > 0 ? <div className="my-1 h-px bg-line" role="separator" /> : null}
          <button
            type="button"
            role="menuitem"
            disabled={item.disabled}
            onClick={() => {
              item.onSelect();
              setState(null);
            }}
            className={cx(
              'flex w-full items-center gap-2 px-2.5 py-1.5 text-left text-[13px] transition-colors',
              'disabled:cursor-not-allowed disabled:opacity-40',
              item.danger ? 'text-danger hover:bg-danger-soft' : 'text-ink hover:bg-surface-sunken',
            )}
          >
            {item.icon ? <span className="text-ink-faint">{item.icon}</span> : null}
            {item.label}
          </button>
        </div>
      ))}
    </div>
  ) : null;

  return {
    open: (x, y, items) => setState({ x, y, items }),
    node,
  };
}
