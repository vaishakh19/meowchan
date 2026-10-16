/**
 * Settings.
 *
 * Every switch here changes real backend behaviour: scan depth, link following, confirmation
 * requirements, and the folders Diskora is allowed to treat as temporary or cache space. The
 * excluded paths list is the safety net for cleanup.
 */

import { useState } from 'react';
import { useApp } from '@/app/state';
import {
  Button,
  Notice,
  Panel,
  SectionTitle,
  Select,
  Switch,
  TextField,
  cx,
} from '@/components/ui/primitives';
import { formatCount } from '@/lib/format';
import type { ThemeMode } from '@/types/contracts';

const CHILD_OPTIONS = [50, 100, 250, 500];

export function SettingsView(): React.JSX.Element {
  const { state, actions } = useApp();
  const settings = state.settings;
  const [cacheDraft, setCacheDraft] = useState('');
  const [tempDraft, setTempDraft] = useState('');
  const [excludeDraft, setExcludeDraft] = useState('');

  if (!settings) {
    return (
      <div className="p-4">
        <Panel className="p-4">
          <p className="text-xs text-ink-muted">Loading settings…</p>
        </Panel>
      </div>
    );
  }

  const patch = (next: Partial<typeof settings>) => void actions.updateSettings(next);

  const addPath = (key: 'cachePaths' | 'tempPaths' | 'excludePaths', value: string) => {
    const trimmed = value.trim();
    if (!trimmed) return;
    const existing = settings[key];
    if (existing.includes(trimmed)) return;
    patch({ [key]: [...existing, trimmed] } as Partial<typeof settings>);
  };

  return (
    <div className="min-h-0 flex-1 overflow-y-auto" data-testid="settings-view">
      <header className="border-b border-line bg-surface px-4 py-3">
        <h1 className="text-[13px] font-semibold text-ink">Settings</h1>
        <p className="mt-0.5 text-[10px] text-ink-faint">
          Stored locally on this PC. Nothing here is synced or uploaded.
        </p>
      </header>

      <div className="max-w-3xl px-4 py-3">
        {/* -------------------------------- appearance -------------------------------- */}
        <Panel className="mb-3 p-3">
          <SectionTitle className="px-0 pt-0">Appearance</SectionTitle>
          <div className="flex flex-wrap items-end gap-4">
            <label className="block">
              <span className="mb-1 block text-[11px] font-medium text-ink-muted">Theme</span>
              <Select
                value={settings.theme}
                onChange={(event) => patch({ theme: event.target.value as ThemeMode })}
                className="h-8 w-40 text-xs"
              >
                <option value="system">Follow Windows</option>
                <option value="light">Light</option>
                <option value="dark">Dark</option>
              </Select>
            </label>
          </div>
          <p className="mt-2 text-[10px] leading-relaxed text-ink-faint">
            Diskora renders at 100, 125 and 150% display scaling. The window is resizable from
            960×640 upwards and all three panes scroll independently.
          </p>
        </Panel>

        {/* --------------------------------- scanning --------------------------------- */}
        <Panel className="mb-3 p-3">
          <SectionTitle className="px-0 pt-0">Scanning</SectionTitle>
          <div className="grid gap-3 sm:grid-cols-2">
            <label className="block">
              <span className="mb-1 block text-[11px] font-medium text-ink-muted">
                Children kept per folder
              </span>
              <Select
                value={String(settings.maxChildrenPerNode)}
                onChange={(event) => patch({ maxChildrenPerNode: Number(event.target.value) })}
                className="h-8 w-full text-xs"
              >
                {CHILD_OPTIONS.map((value) => (
                  <option key={value} value={value}>
                    {value} (folders beyond this are folded into “Other”)
                  </option>
                ))}
              </Select>
            </label>
            <TextField
              label="Default scan root"
              placeholder="C:\\"
              value={settings.defaultScanRoot ?? ''}
              onChange={(event) => patch({ defaultScanRoot: event.target.value || null })}
              className="h-8 font-mono text-[11px]"
            />
          </div>
          <div className="mt-3">
            <Switch
              checked={settings.followLinks}
              onChange={(value) => patch({ followLinks: value })}
              label="Follow junctions and symlinks"
              detail="Off by default. Following them can double-count space and loop; Diskora detects and reports cycles when it is on."
            />
          </div>
        </Panel>

        {/* --------------------------------- cleanup --------------------------------- */}
        <Panel className="mb-3 p-3">
          <SectionTitle className="px-0 pt-0">Cleanup</SectionTitle>
          <Switch
            checked={settings.confirmBeforeCleanup}
            onChange={(value) => patch({ confirmBeforeCleanup: value })}
            label="Confirm before every cleanup action"
            detail="Keeps the exact path list on screen before anything is moved."
          />
          <Switch
            checked={settings.preferRecycleBin}
            onChange={(value) => patch({ preferRecycleBin: value })}
            label="Prefer the Recycle Bin"
            detail="Permanent deletion stays available but is always a separate, clearly-labelled action."
          />
          <label className="mt-3 block max-w-xs">
            <span className="mb-1 block text-[11px] font-medium text-ink-muted">
              “Large unused file” age threshold
            </span>
            <Select
              value={String(settings.largeUnusedDays)}
              onChange={(event) => patch({ largeUnusedDays: Number(event.target.value) })}
              className="h-8 w-full text-xs"
            >
              <option value={30}>30 days</option>
              <option value={90}>90 days</option>
              <option value={180}>180 days</option>
              <option value={365}>a year</option>
            </Select>
          </label>
        </Panel>

        {/* ----------------------------- safety boundaries ----------------------------- */}
        <Panel className="mb-3 p-3">
          <SectionTitle className="px-0 pt-0">Safety boundaries</SectionTitle>
          <Notice tone="info" className="mb-3">
            Cleanup only ever acts inside the folders you list below. Everything else is reported
            and never offered for deletion.
          </Notice>
          <div className="grid gap-3 sm:grid-cols-3">
            <PathList
              title="Cache folders"
              description="Diskora may suggest these for cleanup."
              paths={settings.cachePaths}
              draft={cacheDraft}
              onDraft={setCacheDraft}
              onAdd={() => {
                addPath('cachePaths', cacheDraft);
                setCacheDraft('');
              }}
              onRemove={(path) =>
                patch({ cachePaths: settings.cachePaths.filter((item) => item !== path) })
              }
            />
            <PathList
              title="Temporary folders"
              description="Diskora may suggest these for cleanup."
              paths={settings.tempPaths}
              draft={tempDraft}
              onDraft={setTempDraft}
              onAdd={() => {
                addPath('tempPaths', tempDraft);
                setTempDraft('');
              }}
              onRemove={(path) => patch({ tempPaths: settings.tempPaths.filter((item) => item !== path) })}
            />
            <PathList
              title="Never touch"
              description="Hard block: cleanup refuses these paths."
              paths={settings.excludePaths}
              draft={excludeDraft}
              onDraft={setExcludeDraft}
              onAdd={() => {
                addPath('excludePaths', excludeDraft);
                setExcludeDraft('');
              }}
              onRemove={(path) =>
                patch({ excludePaths: settings.excludePaths.filter((item) => item !== path) })
              }
            />
          </div>
        </Panel>

        {/* --------------------------------- privacy --------------------------------- */}
        <Panel className="mb-3 p-3">
          <SectionTitle className="px-0 pt-0">Privacy</SectionTitle>
          <Switch
            checked={settings.crashReporting}
            onChange={(value) => patch({ crashReporting: value })}
            label="Send anonymous crash reports"
            detail="Off by default. Only a stack trace and app version — never file paths, file names or scan results. Requires Diskora Pro."
          />
          <p className="mt-2 text-[10px] leading-relaxed text-ink-faint">
            No account, no cloud sync, no background scanning, no analytics on full paths.{' '}
            <button type="button" className="underline hover:text-ink" onClick={() => actions.navigate('privacy')}>
              See exactly what is stored
            </button>
            .
          </p>
        </Panel>

        {/* -------------------------------- diagnostics -------------------------------- */}
        <Panel className="p-3">
          <SectionTitle className="px-0 pt-0">Diagnostics</SectionTitle>
          <dl className="dw-tnum grid grid-cols-2 gap-2 text-[11px] sm:grid-cols-4">
            <Fact label="Backend" value={state.transportKind === 'tauri' ? 'native (Rust)' : 'browser sample data'} />
            <Fact label="App version" value={state.appVersion?.version ?? '—'} />
            <Fact label="Platform" value={state.appVersion ? `${state.appVersion.platform} ${state.appVersion.arch}` : '—'} />
            <Fact
              label="Can modify files"
              value={state.canMutate ? 'yes' : 'no — sample data'}
            />
          </dl>
          {state.canMutate ? null : (
            <Notice tone="warn" className="mt-2">
              The preview you are looking at runs in a browser without the native backend, so cleanup,
              reveal and duplicate hashing are simulated. Inside the packaged desktop app they operate
              on the real filesystem.
            </Notice>
          )}
        </Panel>
      </div>
    </div>
  );
}

function PathList({
  title,
  description,
  paths,
  draft,
  onDraft,
  onAdd,
  onRemove,
}: {
  title: string;
  description: string;
  paths: string[];
  draft: string;
  onDraft: (value: string) => void;
  onAdd: () => void;
  onRemove: (path: string) => void;
}): React.JSX.Element {
  return (
    <div>
      <p className="text-[11px] font-medium text-ink">{title}</p>
      <p className="mb-1.5 text-[10px] leading-relaxed text-ink-faint">{description}</p>
      <ul className="mb-1.5 flex flex-col gap-1">
        {paths.length === 0 ? (
          <li className="text-[10px] italic text-ink-faint">none</li>
        ) : (
          paths.map((path) => (
            <li
              key={path}
              className="flex items-center gap-1 rounded-control border border-line bg-surface-sunken px-1.5 py-0.5"
            >
              <span className="min-w-0 flex-1 truncate font-mono text-[10px] text-ink-muted" title={path}>
                {path}
              </span>
              <button
                type="button"
                className="shrink-0 text-[10px] text-ink-faint underline hover:text-danger"
                onClick={() => onRemove(path)}
                aria-label={`Remove ${path}`}
              >
                remove
              </button>
            </li>
          ))
        )}
      </ul>
      <form
        className="flex gap-1"
        onSubmit={(event) => {
          event.preventDefault();
          onAdd();
        }}
      >
        <input
          value={draft}
          onChange={(event) => onDraft(event.target.value)}
          placeholder="C:\path"
          aria-label={`Add a ${title} path`}
          className="h-7 min-w-0 flex-1 rounded-control border border-line bg-surface px-1.5 font-mono text-[10px] text-ink placeholder:text-ink-faint focus:border-info focus:outline-none"
        />
        <Button type="submit" size="sm" variant="secondary" disabled={draft.trim().length === 0}>
          Add
        </Button>
      </form>
      <p className="mt-1 text-[10px] text-ink-faint">{formatCount(paths.length)} configured</p>
    </div>
  );
}

function Fact({ label, value }: { label: string; value: string }) {
  return (
    <div className={cx('min-w-0')}>
      <dt className="text-[10px] text-ink-faint">{label}</dt>
      <dd className="truncate text-[11px] text-ink" title={value}>
        {value}
      </dd>
    </div>
  );
}

