import { useApp } from './state';
export function TitleBar(): React.JSX.Element {
  const { state } = useApp();
  return <header className="flex h-9 shrink-0 items-center justify-between border-b border-line bg-surface px-3" data-testid="title-bar">
    <span className="text-xs font-semibold">Diskora · Disk Space Analyzer</span>
    <span className="text-xs text-ink-faint">Read-only · {state.appVersion?.version ?? '0.1.0'}</span>
  </header>;
}
