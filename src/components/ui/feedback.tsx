/**
 * Overlays: modal dialog, toast host and a small positioned tooltip.
 *
 * The modal is the gate in front of every destructive action — it is the only place in the app
 * where a deletion can be confirmed, and it always shows the exact paths and the total size.
 */

import { useEffect, useRef, type ReactNode } from 'react';
import { AlertTriangle, CheckCircle2, Info, X, XCircle } from 'lucide-react';
import { Button, IconButton, cx, Divider } from './primitives';
import type { Toast } from '@/app/state';

/* ------------------------------------------------------------------ *
 * Modal
 * ------------------------------------------------------------------ */

export function Modal({
  open,
  title,
  description,
  onClose,
  children,
  footer,
  tone = 'neutral',
  width = 'max-w-lg',
}: {
  open: boolean;
  title: string;
  description?: ReactNode;
  onClose: () => void;
  children?: ReactNode;
  footer?: ReactNode;
  tone?: 'neutral' | 'danger';
  width?: string;
}) {
  const dialogRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape') onClose();
      if (event.key === 'Tab' && dialogRef.current) {
        // Trap focus inside the dialog.
        const focusable = dialogRef.current.querySelectorAll<HTMLElement>(
          'button, [href], input, select, textarea, [tabindex]:not([tabindex="-1"])',
        );
        if (focusable.length === 0) return;
        const first = focusable[0];
        const last = focusable[focusable.length - 1];
        if (!event.shiftKey && document.activeElement === last) {
          event.preventDefault();
          first.focus();
        } else if (event.shiftKey && document.activeElement === first) {
          event.preventDefault();
          last.focus();
        }
      }
    };
    document.addEventListener('keydown', onKeyDown);
    dialogRef.current?.querySelector<HTMLElement>('button, input')?.focus();
    return () => document.removeEventListener('keydown', onKeyDown);
  }, [open, onClose]);

  if (!open) return null;

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center p-6">
      <div
        className="absolute inset-0 bg-ink-strong/35 backdrop-blur-[2px]"
        onClick={onClose}
        aria-hidden
      />
      <div
        ref={dialogRef}
        role="dialog"
        aria-modal="true"
        aria-label={title}
        className={cx(
          'relative flex max-h-[85vh] w-full flex-col overflow-hidden rounded-panel border border-line bg-surface shadow-drag',
          width,
        )}
      >
        <header className="flex items-start justify-between gap-3 px-4 pt-3.5 pb-2">
          <div className="min-w-0">
            <h2 className="flex items-center gap-2 text-[15px] font-semibold text-ink">
              {tone === 'danger' ? <AlertTriangle aria-hidden className="size-4 text-danger" /> : null}
              {title}
            </h2>
            {description ? (
              <p className="mt-1 text-xs leading-relaxed text-ink-muted">{description}</p>
            ) : null}
          </div>
          <IconButton label="Close dialog" onClick={onClose}>
            <X aria-hidden className="size-4" />
          </IconButton>
        </header>
        <Divider />
        <div className="min-h-0 flex-1 overflow-y-auto px-4 py-3">{children}</div>
        {footer ? (
          <>
            <Divider />
            <footer className="flex items-center justify-end gap-2 bg-surface-sunken/60 px-4 py-2.5">{footer}</footer>
          </>
        ) : null}
      </div>
    </div>
  );
}

/* ------------------------------------------------------------------ *
 * Toasts
 * ------------------------------------------------------------------ */

const TOAST_ICONS = {
  info: Info,
  success: CheckCircle2,
  warning: AlertTriangle,
  error: XCircle,
} as const;

const TOAST_TONES = {
  info: 'text-info',
  success: 'text-ok',
  warning: 'text-warn',
  error: 'text-danger',
} as const;

export function ToastHost({
  toasts,
  onDismiss,
}: {
  toasts: Toast[];
  onDismiss: (id: string) => void;
}) {
  return (
    <div
      className="pointer-events-none fixed bottom-3 right-3 z-40 flex w-80 flex-col gap-2"
      role="region"
      aria-label="Notifications"
    >
      {toasts.map((toast) => {
        const Icon = TOAST_ICONS[toast.kind];
        return (
          <div
            key={toast.id}
            role="status"
            aria-live={toast.kind === 'error' ? 'assertive' : 'polite'}
            className="pointer-events-auto flex items-start gap-2 rounded-panel border border-line bg-surface px-3 py-2.5 shadow-pop"
          >
            <Icon aria-hidden className={cx('mt-px size-4 shrink-0', TOAST_TONES[toast.kind])} />
            <div className="min-w-0 flex-1">
              <p className="text-[13px] font-medium leading-snug text-ink">{toast.title}</p>
              {toast.detail ? (
                <p className="mt-0.5 break-words text-[11px] leading-relaxed text-ink-muted">{toast.detail}</p>
              ) : null}
              {toast.action ? (
                <Button
                  size="sm"
                  variant="ghost"
                  className="mt-1 -ml-1 h-6 px-1.5 text-[11px]"
                  onClick={() => {
                    toast.action?.run();
                    onDismiss(toast.id);
                  }}
                >
                  {toast.action.label}
                </Button>
              ) : null}
            </div>
            <IconButton label="Dismiss" onClick={() => onDismiss(toast.id)}>
              <X aria-hidden className="size-3.5" />
            </IconButton>
          </div>
        );
      })}
    </div>
  );
}

/* ------------------------------------------------------------------ *
 * Tooltip (chart overlays)
 * ------------------------------------------------------------------ */

export interface TooltipPlacement {
  x: number;
  y: number;
}

/**
 * A floating tooltip anchored to a point inside `container`. It flips when it would overflow the
 * container edges, which is what keeps chart tooltips usable at any window size or DPI.
 */
export function FloatingTooltip({
  anchor,
  container,
  children,
  visible,
}: {
  anchor: TooltipPlacement;
  container: { width: number; height: number };
  children: ReactNode;
  visible: boolean;
}) {
  if (!visible) return null;
  const width = 260;
  const height = 96;
  const flipX = anchor.x + width + 16 > container.width;
  const flipY = anchor.y + height + 12 > container.height;
  const left = flipX ? Math.max(8, anchor.x - width - 12) : anchor.x + 12;
  const top = flipY ? Math.max(8, anchor.y - height - 12) : anchor.y + 12;
  return (
    <div
      role="tooltip"
      className="pointer-events-none absolute z-30 max-w-[260px] rounded-control border border-line bg-surface px-2.5 py-2 shadow-pop"
      style={{ left, top, width }}
    >
      {children}
    </div>
  );
}
