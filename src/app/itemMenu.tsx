/**
 * Item context menu.
 *
 * The menu is defined in one place so every surface (sunburst, treemap, file list, large files,
 * duplicates) offers exactly the same safe actions. There is no delete entry: destructive work
 * happens only in the cleanup assistant, behind a confirmation dialog that lists the exact paths.
 */

import { createContext, useCallback, useContext, useMemo, type ReactNode } from 'react';
import { Copy, Crosshair, FolderOpen, ExternalLink, ShieldOff } from 'lucide-react';
import { useContextMenu, type ContextMenuItem } from '@/components/ui/ContextMenu';
import { useApp } from './state';
import { isSyntheticPath } from '@/lib/tree';

interface ItemMenuApi {
  /** Opens the menu for `path` at viewport coordinates. */
  open(path: string, x: number, y: number, kind?: 'file' | 'folder'): void;
}

const ItemMenuContext = createContext<ItemMenuApi | null>(null);

export function ItemMenuProvider({ children }: { children: ReactNode }): React.JSX.Element {
  const { state, actions } = useApp();
  const menu = useContextMenu();

  const open = useCallback(
    (path: string, x: number, y: number, kind: 'file' | 'folder' = 'folder') => {
      const synthetic = isSyntheticPath(path);
      const items: ContextMenuItem[] = [
        {
          label: 'Reveal in File Explorer',
          icon: <FolderOpen aria-hidden className="size-3.5" />,
          disabled: synthetic,
          onSelect: () => void actions.reveal(path),
        },
        {
          label: kind === 'file' ? 'Open with default app' : 'Open folder',
          icon: <ExternalLink aria-hidden className="size-3.5" />,
          disabled: synthetic,
          onSelect: () => void actions.open(path),
        },
        {
          label: 'Copy full path',
          icon: <Copy aria-hidden className="size-3.5" />,
          onSelect: () => void actions.copyPath(path),
        },
        {
          label: 'Focus in visualization',
          icon: <Crosshair aria-hidden className="size-3.5" />,
          disabled: kind !== 'folder',
          separatorBefore: true,
          onSelect: () => {
            actions.navigate('analyze');
            actions.focusPath(path);
          },
        },

      ];

      if (state.route === 'duplicates') {
        items.push({
          label: 'Exclude this folder from duplicate scans',
          icon: <ShieldOff aria-hidden className="size-3.5" />,
          separatorBefore: true,
          onSelect: () => actions.excludeDuplicatePath(path),
        });
      }

      menu.open(x, y, items);
    },
    [actions, menu, state.route],
  );

  const value = useMemo<ItemMenuApi>(() => ({ open }), [open]);

  return (
    <ItemMenuContext.Provider value={value}>
      {children}
      {menu.node}
    </ItemMenuContext.Provider>
  );
}

export function useItemMenu(): ItemMenuApi {
  const context = useContext(ItemMenuContext);
  if (!context) throw new Error('useItemMenu must be used inside <ItemMenuProvider>.');
  return context;
}
