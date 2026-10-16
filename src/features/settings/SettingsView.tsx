import { useApp } from '@/app/state';
import { Panel, SectionTitle, Switch } from '@/components/ui/primitives';

export function SettingsView(): React.JSX.Element {
  const { state, actions } = useApp();
  const settings = state.settings;
  if (!settings) return <p className="p-3">Settings are unavailable. Restart Diskora and check that your application data directory is writable.</p>;
  return <div className="overflow-y-auto p-3" data-testid="settings-view">
    <h1 className="mb-3 font-semibold">Settings</h1>
    <Panel className="space-y-3 p-3">
      <SectionTitle>Appearance and scanning</SectionTitle>
      <label className="block">Theme <select aria-label="Theme" value={settings.theme}
        onChange={event => void actions.updateSettings({ theme: event.target.value as typeof settings.theme })}>
        <option value="system">System</option><option value="light">Light</option><option value="dark">Dark</option>
      </select></label>
      <Switch checked={settings.followLinks} onChange={followLinks => void actions.updateSettings({ followLinks })}
        label="Follow junctions and symlinks" detail="Off by default. Following links can scan outside the selected folder; cycles are detected." />
      <p className="text-xs text-ink-muted">Diskora is a read-only analyzer. No accounts, telemetry, file deletion, or background services.</p>
    </Panel>
    <p className="mt-3 text-xs text-ink-muted">Version {state.appVersion?.version ?? 'unknown'} · {state.appVersion?.platform} {state.appVersion?.arch}</p>
  </div>;
}
