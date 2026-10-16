/**
 * Custom title bar.
 *
 * Tauri windows are created with `decorations: false`, so the app draws its own frame: a drag
 * region on the left, the product identity in the middle, and Windows-standard caption buttons on
 * the right. In a plain browser there is no window to control, so the frame degrades to a static
 * header with a preview badge.
 */

import { useEffect, useState } from 'react';
import { Minus, Square, Copy, X } from 'lucide-react';
import { IconButton, Badge, cx } from '@/components/ui/primitives';
import { useApp } from './state';
import { isTauriRuntime } from '@/lib/transport';

async function windowApi() {
  return import('@tauri-apps/api/window');
}

export function TitleBar(): React.JSX.Element {
  const { state } = useApp();
  const [maximized, setMaximized] = useState(false);
  const native = isTauriRuntime();

  useEffect(() => {
    if (!native) return;
    let unlisten: (() => void) | undefined;
    void (async () => {
      const api = await windowApi();
      const current = api.getCurrentWindow();
      setMaximized(await current.isMaximized());
      unlisten = await current.onResized(async () => {
        setMaximized(await current.isMaximized());
      });
    })();
    return () => unlisten?.();
  }, [native]);

  return (
    <header
      className="dw-titlebar-drag flex h-9 shrink-0 items-center justify-between border-b border-line bg-surface px-2"
      data-testid="title-bar"
    >
      <div className="flex min-w-0 items-center gap-2">
        <AppMark className="size-4" />
        <span className="text-[12px] font-semibold tracking-tight text-ink">Diskora</span>
        <span className="text-[11px] text-ink-faint">Disk Space Analyzer</span>
        {!native ? (
          <Badge tone="warn" className="ml-1">
            Browser preview · synthetic data
          </Badge>
        ) : null}
      </div>

      <div className="flex items-center gap-2">
        {state.appVersion ? (
          <span className="dw-tnum text-[10px] text-ink-faint">v{state.appVersion.version}</span>
        ) : null}
        {state.entitlement.state === 'pro-active' || state.entitlement.state === 'trial' ? (
          <Badge tone="accent">Pro</Badge>
        ) : (
          <Badge tone="neutral">Free</Badge>
        )}
      </div>

      {native ? (
        <div className="dw-no-drag flex items-center gap-0.5">
          <IconButton
            label="Minimize"
            onClick={() => void windowApi().then((api) => api.getCurrentWindow().minimize())}
          >
            <Minus aria-hidden className="size-3.5" />
          </IconButton>
          <IconButton
            label={maximized ? 'Restore' : 'Maximize'}
            onClick={() =>
              void windowApi().then((api) => api.getCurrentWindow().toggleMaximize())
            }
          >
            {maximized ? <Copy aria-hidden className="size-3" /> : <Square aria-hidden className="size-3" />}
          </IconButton>
          <IconButton
            label="Close"
            className="hover:bg-danger-soft hover:text-danger"
            onClick={() => void windowApi().then((api) => api.getCurrentWindow().close())}
          >
            <X aria-hidden className="size-3.5" />
          </IconButton>
        </div>
      ) : (
        <div className="w-16" />
      )}
    </header>
  );
}

/** The Diskora mark: a storage ring with three usage arcs. */
export function AppMark({ className }: { className?: string }): React.JSX.Element {
  return (
    <svg
      viewBox="0 0 24 24"
      className={cx('shrink-0', className)}
      role="img"
      aria-label="Diskora"
      fill="none"
    >
      <circle cx="12" cy="12" r="9" stroke="currentColor" strokeWidth="1.6" className="text-line-strong" />
      <path
        d="M12 3a9 9 0 0 1 8.49 6.02"
        stroke="#a0c0d9"
        strokeWidth="3.4"
        strokeLinecap="round"
      />
      <path d="M20.49 9.02A9 9 0 0 1 12 21" stroke="#d9a0ac" strokeWidth="3.4" strokeLinecap="round" />
      <path d="M12 21a9 9 0 0 1-6.36-2.64" stroke="#c8a0d9" strokeWidth="3.4" strokeLinecap="round" />
      <circle cx="12" cy="12" r="2.1" fill="currentColor" className="text-ink" />
    </svg>
  );
}
