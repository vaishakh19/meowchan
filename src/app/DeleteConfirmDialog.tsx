/**
 * The one gate in front of every deletion.
 *
 * Diskora only deletes after this dialog has listed the exact items —
 * name, full path and size — and the user has confirmed. The actual
 * move to the Windows Recycle Bin happens in the native backend; this
 * dialog never touches the filesystem.
 */

import { useEffect, useMemo, useState } from 'react';
import { AlertTriangle, File, Folder, Loader2 } from 'lucide-react';
import { useApp } from './state';
import { Button, Panel } from '@/components/ui/primitives';
import { Modal } from '@/components/ui/feedback';
import { formatBytes, formatBytesExact, formatCount } from '@/lib/format';
import { findNode } from '@/features/visualization/layout';
import type { FileDetails, TreeNode } from '@/types/contracts';

interface ResolvedItem {
  path: string;
  name: string;
  kind: 'file' | 'folder';
  sizeBytes: number;
  fileCount: number;
  folderCount: number;
}

export function DeleteConfirmDialog(): React.JSX.Element | null {
  const { state, actions } = useApp();
  const { paths, working } = state.deletion;

  // Metadata comes from the scan tree first (it matches the numbers the
  // user has been looking at) and from live details for anything the
  // pruned tree no longer contains — e.g. items picked in the Duplicates
  // or Large Files views.
  const [liveDetails, setLiveDetails] = useState<Map<string, FileDetails>>(new Map());

  const tree = state.scan.result?.tree ?? null;

  const missing = useMemo(
    () => (paths ?? []).filter((path) => !tree || !findNode(tree, path)),
    [paths, tree],
  );

  useEffect(() => {
    if (missing.length === 0) return;
    let cancelled = false;
    void (async () => {
      const fetched = new Map<string, FileDetails>();
      for (const path of missing) {
        try {
          // The transport is available whenever a scan result is.
          const { getTransport } = await import('@/lib/transport');
          fetched.set(path, await (await getTransport()).fileDetails(path));
        } catch {
          // Unreadable items are shown with the information the scan
          // already has; the backend re-validates everything anyway.
        }
      }
      if (!cancelled) setLiveDetails((previous) => new Map([...previous, ...fetched]));
    })();
    return () => {
      cancelled = true;
    };
  }, [missing]);

  if (!paths || paths.length === 0) return null;

  const items: ResolvedItem[] = paths.map((path) => {
    const node = tree ? findNode(tree, path) : null;
    const live = liveDetails.get(path);
    const source: TreeNode | FileDetails | null = node ?? live ?? null;
    return {
      path,
      name: source?.name ?? path.split('\\').pop() ?? path,
      kind: (source?.kind ?? 'file') as 'file' | 'folder',
      sizeBytes: source?.sizeBytes ?? 0,
      fileCount: source?.kind === 'folder' ? (source as TreeNode).fileCount : 0,
      folderCount: source?.kind === 'folder' ? (source as TreeNode).folderCount : 0,
    };
  });

  const totalBytes = items.reduce((sum, item) => sum + item.sizeBytes, 0);

  return (
    <Modal
      open
      tone="danger"
      title="Move to the Recycle Bin?"
      description={
        items.length === 1
          ? 'The exact item is listed below. It can be restored from the Recycle Bin.'
          : `${items.length} items are listed below. They can be restored from the Recycle Bin.`
      }
      onClose={actions.cancelDeletion}
      footer={
        <>
          <Button variant="secondary" onClick={actions.cancelDeletion} disabled={working}>
            Cancel
          </Button>
          <Button
            variant="danger"
            onClick={() => void actions.confirmDeletion()}
            disabled={working}
            loading={working}
            data-testid="confirm-deletion"
          >
            Move to Recycle Bin
          </Button>
        </>
      }
    >
      <ul className="flex flex-col gap-2" data-testid="deletion-items">
        {items.map((item) => (
          <li key={item.path}>
            <Panel className="p-2.5">
              <div className="flex items-start gap-2.5">
                <span className="mt-0.5 text-ink-faint">
                  {item.kind === 'folder' ? (
                    <Folder aria-hidden className="size-4" />
                  ) : (
                    <File aria-hidden className="size-4" />
                  )}
                </span>
                <div className="min-w-0 flex-1">
                  <p className="break-words text-[13px] font-medium text-ink">{item.name}</p>
                  <p className="mt-0.5 break-all font-mono text-[10px] leading-relaxed text-ink-muted">
                    {item.path}
                  </p>
                  <p className="dw-tnum mt-1 flex flex-wrap gap-x-3 text-[10px] text-ink-faint">
                    <span>{formatBytes(item.sizeBytes)} ({formatBytesExact(item.sizeBytes)} exact)</span>
                    {item.kind === 'folder' ? (
                      <span>
                        {formatCount(item.fileCount)} files · {formatCount(item.folderCount)} folders
                      </span>
                    ) : null}
                  </p>
                </div>
              </div>
            </Panel>
          </li>
        ))}
      </ul>

      <p className="dw-tnum mt-3 text-[12px] text-ink">
        Total: {formatBytes(totalBytes)} across {items.length} {items.length === 1 ? 'item' : 'items'}
      </p>

      <p className="mt-2 flex items-start gap-2 rounded-control bg-warn-soft px-2.5 py-2 text-[11px] leading-relaxed text-ink">
        <AlertTriangle aria-hidden className="mt-px size-3.5 shrink-0 text-warn" />
        Items are moved to the Windows Recycle Bin — recoverable until it is
        emptied. Folders move with everything inside them. Diskora cannot
        delete through links or junctions, and never deletes permanently.
      </p>

      {working ? (
        <p className="mt-2 flex items-center gap-1.5 text-[11px] text-ink-muted" role="status">
          <Loader2 aria-hidden className="size-3.5 animate-spin" />
          Moving to the Recycle Bin…
        </p>
      ) : null}
    </Modal>
  );
}
