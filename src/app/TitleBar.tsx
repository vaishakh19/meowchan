import { Crown } from 'lucide-react';
import { useApp } from './state';
import { Badge, Button } from '@/components/ui/primitives';

export function TitleBar(): React.JSX.Element {
  const { state, actions } = useApp();
  const pro = state.pro;
  return (
    <header className="flex h-9 shrink-0 items-center justify-between border-b border-line bg-surface px-3" data-testid="title-bar">
      <span className="text-xs font-semibold">Diskora · Disk Space Analyzer</span>
      <span className="flex items-center gap-2">
        {pro?.enabled ? (
          <Badge tone="accent" className="text-[9px]" data-testid="pro-badge">
            <Crown aria-hidden className="size-2.5" />
            Pro
          </Badge>
        ) : (
          <Button
            size="sm"
            variant="secondary"
            className="h-6 px-2 text-[11px]"
            onClick={() => actions.navigate('pro')}
            title="Upgrade to Diskora Pro — $12.99, one-time"
            data-testid="upgrade-to-pro"
          >
            <Crown aria-hidden className="size-3" />
            Upgrade to Pro
          </Button>
        )}
        <span className="text-xs text-ink-faint">{state.appVersion?.version ?? '0.1.0'}</span>
      </span>
    </header>
  );
}
