/**
 * File-type iconography.
 *
 * Extension → icon + tint. Deliberately coarse: a handful of recognisable shapes beat a wall of
 * near-identical glyphs, and every icon inherits its colour from the storage category so the
 * details panel, the file list and the legend all agree.
 */

import {
  Archive,
  File,
  FileAudio,
  FileCode,
  FileImage,
  FileSpreadsheet,
  FileText,
  FileVideo,
  Folder,
  HardDrive,
  type LucideIcon,
} from 'lucide-react';
import type { StorageCategory } from '@/types/contracts';
import { categoryColor } from '@/lib/categories';

interface IconSpec {
  icon: LucideIcon;
  /** Tint applied on top of the category colour, so the shape stays readable. */
  tint?: string;
}

const VIDEO = new Set(['mp4', 'mkv', 'avi', 'mov', 'wmv', 'flv', 'webm', 'm4v', 'mpg', 'mpeg', '3gp']);
const AUDIO = new Set(['mp3', 'flac', 'wav', 'aac', 'ogg', 'wma', 'm4a', 'aiff', 'opus']);
const IMAGE = new Set(['jpg', 'jpeg', 'png', 'gif', 'bmp', 'tiff', 'heic', 'webp', 'svg', 'raw', 'arw', 'cr2', 'nef', 'dng']);
const ARCHIVE = new Set(['zip', 'rar', '7z', 'tar', 'gz', 'bz2', 'xz', 'iso', 'cab', 'wim', 'dmg']);
const CODE = new Set([
  'js', 'jsx', 'ts', 'tsx', 'rs', 'py', 'rb', 'go', 'java', 'c', 'h', 'cpp', 'hpp', 'cs', 'php',
  'html', 'css', 'scss', 'json', 'toml', 'yaml', 'yml', 'xml', 'sh', 'ps1', 'bat', 'cmd', 'sql',
  'ipynb', 'vue', 'svelte',
]);
const DOC = new Set(['doc', 'docx', 'odt', 'rtf', 'txt', 'md', 'pdf']);
const SHEET = new Set(['xls', 'xlsx', 'ods', 'csv', 'tsv']);
const DATA = new Set(['db', 'sqlite', 'sqlite3', 'mdb', 'accdb', 'dat', 'pack', 'idx']);

export function iconForExtension(extension: string | null): IconSpec {
  if (!extension) return { icon: File };
  if (VIDEO.has(extension)) return { icon: FileVideo };
  if (AUDIO.has(extension)) return { icon: FileAudio };
  if (IMAGE.has(extension)) return { icon: FileImage };
  if (ARCHIVE.has(extension)) return { icon: Archive };
  if (CODE.has(extension)) return { icon: FileCode };
  if (DOC.has(extension)) return { icon: FileText };
  if (SHEET.has(extension)) return { icon: FileSpreadsheet };
  if (DATA.has(extension)) return { icon: HardDrive };
  return { icon: File };
}

export interface FileIconProps {
  name: string;
  extension: string | null;
  kind: 'file' | 'folder';
  category: StorageCategory;
  className?: string;
  /** Drives and roots get the drive glyph. */
  isRoot?: boolean;
}

export function FileIcon({
  name,
  extension,
  kind,
  category,
  className,
  isRoot = false,
}: FileIconProps): React.JSX.Element {
  const spec =
    kind === 'folder' || isRoot
      ? { icon: isRoot ? HardDrive : Folder }
      : iconForExtension(extension);
  const Icon = spec.icon;
  const color = kind === 'folder' || isRoot ? categoryColor(category) : 'var(--color-ink-faint)';
  return (
    <Icon
      aria-hidden
      className={className}
      style={{ color }}
      data-file-icon={name}
      strokeWidth={1.75}
    />
  );
}

/** Human label for an extension, e.g. `mp4` → `MP4 video`. */
export function extensionLabel(extension: string | null): string {
  if (!extension) return 'No extension';
  return extension.toUpperCase();
}
