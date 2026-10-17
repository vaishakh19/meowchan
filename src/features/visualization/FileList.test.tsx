import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { FileList } from './FileList';
import type { ChildrenPage, TreeNode } from '@/types/contracts';

const { getTransport } = vi.hoisted(() => ({ getTransport: vi.fn() }));
vi.mock('@/lib/transport', () => ({ getTransport }));

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

const root: TreeNode = {
  id: 'root',
  path: 'C:\\data',
  name: 'data',
  kind: 'folder',
  sizeBytes: 30,
  diskSizeBytes: null,
  fileCount: 2,
  folderCount: 0,
  modifiedAt: null,
  createdAt: null,
  extension: null,
  category: 'other',
  accessDenied: false,
  children: [],
  synthetic: false,
};

function file(path: string, name: string, sizeBytes: number): TreeNode {
  return {
    ...root,
    id: path,
    path,
    name,
    kind: 'file',
    sizeBytes,
    fileCount: 0,
    folderCount: 0,
    extension: 'txt',
    children: [],
  };
}

function page(path: string, offset: number, items: TreeNode[], total: number): ChildrenPage {
  return { path, offset, limit: 500, items, total };
}

describe('paged FileList', () => {
  it('loads a bounded first page and requests the next page without losing its query', async () => {
    const scanChildren = vi.fn()
      .mockResolvedValueOnce(page(root.path, 0, [file('C:\\data\\a.txt', 'a.txt', 10)], 2))
      .mockResolvedValueOnce(page(root.path, 1, [file('C:\\data\\b.txt', 'b.txt', 20)], 2));
    getTransport.mockResolvedValue({ scanChildren });

    render(
      <FileList
        scanId="scan-1"
        root={root}
        rootPath={root.path}
        scanRootPath={root.path}
        selectedPath={null}
        sort="size"
        sortDir="desc"
        search=""
        onSelect={vi.fn()}
        onOpen={vi.fn()}
        onSortChange={vi.fn()}
        onSearchChange={vi.fn()}
        onContextMenu={vi.fn()}
      />,
    );

    expect(await screen.findByText('a.txt')).toBeDefined();
    expect(scanChildren).toHaveBeenNthCalledWith(1, {
      scanId: 'scan-1', path: root.path, offset: 0, limit: 500,
      sort: 'size', sortDir: 'desc', search: '',
    });
    expect(screen.getByText('1 of 2 items')).toBeDefined();

    fireEvent.click(screen.getByRole('button', { name: 'Load next page' }));
    expect(await screen.findByText('b.txt')).toBeDefined();
    await waitFor(() => expect(scanChildren).toHaveBeenCalledTimes(2));
    expect(scanChildren).toHaveBeenNthCalledWith(2, {
      scanId: 'scan-1', path: root.path, offset: 1, limit: 500,
      sort: 'size', sortDir: 'desc', search: '',
    });
    expect(screen.getByText('2 of 2 items')).toBeDefined();
  });

  it('toggles the active sort and starts a new sort descending', () => {
    const scanChildren = vi.fn().mockResolvedValue(page(root.path, 0, [], 0));
    getTransport.mockResolvedValue({ scanChildren });
    const onSortChange = vi.fn();
    render(
      <FileList
        scanId="scan-sort"
        root={root}
        rootPath={root.path}
        scanRootPath={root.path}
        selectedPath={null}
        sort="size"
        sortDir="desc"
        search=""
        onSelect={vi.fn()}
        onOpen={vi.fn()}
        onSortChange={onSortChange}
        onSearchChange={vi.fn()}
        onContextMenu={vi.fn()}
      />,
    );

    fireEvent.click(screen.getByRole('button', { name: 'Size' }));
    expect(onSortChange).toHaveBeenCalledWith('size', 'asc');
    fireEvent.click(screen.getByRole('button', { name: 'Name' }));
    expect(onSortChange).toHaveBeenCalledWith('name', 'desc');
  });

  it('uses the selected folder path and search for a new bounded query', async () => {
    const nestedPath = 'C:\\data\\nested';
    const searchPage = page(nestedPath, 0, [file(`${nestedPath}\\report.txt`, 'report.txt', 30)], 1);
    const scanChildren = vi.fn().mockResolvedValue(searchPage);
    getTransport.mockResolvedValue({ scanChildren });
    const onSearchChange = vi.fn();

    render(
      <FileList
        scanId="scan-2"
        root={root}
        rootPath={nestedPath}
        scanRootPath={root.path}
        selectedPath={null}
        sort="name"
        sortDir="asc"
        search="report"
        onSelect={vi.fn()}
        onOpen={vi.fn()}
        onSortChange={vi.fn()}
        onSearchChange={onSearchChange}
        onContextMenu={vi.fn()}
      />,
    );

    expect(await screen.findByText('report.txt')).toBeDefined();
    await waitFor(() => expect(scanChildren).toHaveBeenCalledWith({
      scanId: 'scan-2', path: 'C:\\data\\nested', offset: 0, limit: 500,
      sort: 'name', sortDir: 'asc', search: 'report',
    }));
    expect(screen.getByRole('button', { name: 'Open parent folder' })).toBeDefined();
  });
});
