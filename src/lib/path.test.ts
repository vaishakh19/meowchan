import { describe, expect, it } from 'vitest';
import {
  baseName,
  breadcrumbs,
  driveLetter,
  extensionOf,
  isAbsolutePath,
  isSafePathString,
  isUncPath,
  isWithin,
  joinPath,
  normalizePath,
  parentPath,
  pathSegments,
  stripQuotes,
  stripTrailingSeparator,
} from './path';

describe('isAbsolutePath', () => {
  it('recognises drive, UNC and POSIX roots', () => {
    expect(isAbsolutePath('C:\\')).toBe(true);
    expect(isAbsolutePath('c:/Users')).toBe(true);
    expect(isAbsolutePath('\\\\server\\share')).toBe(true);
    expect(isAbsolutePath('/home/user')).toBe(true);
    expect(isAbsolutePath('Users\\Bob')).toBe(false);
    expect(isAbsolutePath('')).toBe(false);
  });

  it('recognises UNC paths', () => {
    expect(isUncPath('\\\\server\\share')).toBe(true);
    expect(isUncPath('C:\\Users')).toBe(false);
  });
});

describe('stripQuotes', () => {
  it('removes shell quoting artefacts', () => {
    expect(stripQuotes('  "C:\\Users"  ')).toBe('C:\\Users');
    expect(stripQuotes("'C:\\Users'")).toBe('C:\\Users');
    expect(stripQuotes('C:\\Users')).toBe('C:\\Users');
  });
});

describe('pathSegments', () => {
  it('splits and resolves dot segments', () => {
    expect(pathSegments('C:\\Users\\Bob\\.\\Downloads')).toEqual(['C:', 'Users', 'Bob', 'Downloads']);
    expect(pathSegments('C:\\Users\\Bob\\..\\Alice')).toEqual(['C:', 'Users', 'Alice']);
    expect(pathSegments('\\\\server\\share\\dir')).toEqual(['server', 'share', 'dir']);
    expect(pathSegments('/usr//local/bin')).toEqual(['usr', 'local', 'bin']);
  });
});

describe('normalizePath', () => {
  it('normalises drive paths', () => {
    expect(normalizePath('c:/users/bob/')).toBe('C:\\users\\bob');
    expect(normalizePath('C:\\Users\\\\Bob')).toBe('C:\\Users\\Bob');
    expect(normalizePath('C:')).toBe('C:\\');
    expect(normalizePath('C:\\')).toBe('C:\\');
  });

  it('normalises UNC paths', () => {
    expect(normalizePath('\\\\SERVER\\share\\dir')).toBe('\\\\SERVER\\share\\dir');
    expect(normalizePath('\\\\SERVER\\share')).toBe('\\\\SERVER\\share');
  });

  it('normalises POSIX paths without rewriting separators', () => {
    expect(normalizePath('/usr/local/bin')).toBe('/usr/local/bin');
    expect(normalizePath('/usr/local/bin/')).toBe('/usr/local/bin');
  });

  it('returns an empty string for empty input', () => {
    expect(normalizePath('')).toBe('');
    expect(normalizePath('   ')).toBe('');
  });
});

describe('baseName / parentPath / extensionOf', () => {
  it('extracts the final component', () => {
    expect(baseName('C:\\Users\\Bob\\Downloads')).toBe('Downloads');
    expect(baseName('C:\\Users\\Bob\\')).toBe('Bob');
    expect(baseName('C:\\')).toBe('C:');
    expect(baseName('file.txt')).toBe('file.txt');
  });

  it('walks up to the parent', () => {
    expect(parentPath('C:\\Users\\Bob')).toBe('C:\\Users');
    expect(parentPath('C:\\Users')).toBe('C:\\');
    expect(parentPath('C:\\')).toBeNull();
    expect(parentPath('\\\\server\\share')).toBeNull();
    expect(parentPath('\\\\server\\share\\dir')).toBe('\\\\server\\share');
    expect(parentPath('/usr/local')).toBe('/usr');
  });

  it('extracts extensions', () => {
    expect(extensionOf('C:\\a\\b.mp4')).toBe('mp4');
    expect(extensionOf('archive.tar.gz')).toBe('gz');
    expect(extensionOf('README')).toBeNull();
    expect(extensionOf('.gitignore')).toBeNull();
    expect(extensionOf('weird.')).toBeNull();
    expect(extensionOf('C:\\dir.with.dots\\file')).toBeNull();
  });
});

describe('joinPath', () => {
  it('joins and normalises', () => {
    expect(joinPath('C:\\Users', 'Bob')).toBe('C:\\Users\\Bob');
    expect(joinPath('C:\\Users\\', 'Bob\\')).toBe('C:\\Users\\Bob');
    expect(joinPath('C:\\', 'Windows')).toBe('C:\\Windows');
    expect(joinPath(null, undefined, '')).toBe('');
  });
});

describe('isWithin', () => {
  it('is segment based, not string-prefix based', () => {
    expect(isWithin('C:\\Users\\Bob\\Downloads', 'C:\\Users')).toBe(true);
    expect(isWithin('C:\\Users\\Bob', 'C:\\Users')).toBe(true);
    // The classic prefix bug: Bobbie is not inside Bob.
    expect(isWithin('C:\\Users\\Bobbie', 'C:\\Users\\Bob')).toBe(false);
    expect(isWithin('C:\\Users\\Alice', 'C:\\Users\\Bob')).toBe(false);
  });

  it('handles drive roots', () => {
    expect(isWithin('C:\\Windows', 'C:\\')).toBe(true);
    expect(isWithin('D:\\Windows', 'C:\\')).toBe(false);
  });

  it('is case insensitive', () => {
    expect(isWithin('c:\\users\\bob', 'C:\\Users')).toBe(true);
  });
});

describe('isSafePathString', () => {
  it('rejects dangerous input', () => {
    expect(isSafePathString('C:\\Users')).toBe(true);
    expect(isSafePathString('')).toBe(false);
    expect(isSafePathString('C:\\Users\u0000evil')).toBe(false);
    expect(isSafePathString('C:\\Users\u0001evil')).toBe(false);
    expect(isSafePathString('x'.repeat(40_000))).toBe(false);
  });
});

describe('driveLetter / stripTrailingSeparator', () => {
  it('extracts drive letters', () => {
    expect(driveLetter('C:\\Users')).toBe('C:');
    expect(driveLetter('c:\\')).toBe('C:');
    expect(driveLetter('\\\\server\\share')).toBeNull();
  });

  it('strips trailing separators but keeps roots', () => {
    expect(stripTrailingSeparator('C:\\Users\\')).toBe('C:\\Users');
    expect(stripTrailingSeparator('C:\\')).toBe('C:\\');
    expect(stripTrailingSeparator('/usr/local/')).toBe('/usr/local');
  });
});

describe('breadcrumbs', () => {
  it('builds a drive trail', () => {
    const crumbs = breadcrumbs('C:\\Users\\Bob\\Downloads');
    expect(crumbs.map((c) => c.label)).toEqual(['C:', 'Users', 'Bob', 'Downloads']);
    expect(crumbs.map((c) => c.path)).toEqual([
      'C:\\',
      'C:\\Users',
      'C:\\Users\\Bob',
      'C:\\Users\\Bob\\Downloads',
    ]);
  });

  it('builds a UNC trail', () => {
    const crumbs = breadcrumbs('\\\\server\\share\\docs\\2026');
    expect(crumbs.map((c) => c.label)).toEqual(['\\\\', 'server\\share', 'docs', '2026']);
  });

  it('builds a POSIX trail', () => {
    const crumbs = breadcrumbs('/home/user/diskora');
    expect(crumbs.map((c) => c.path)).toEqual(['/', '/home', '/home/user', '/home/user/diskora']);
  });
});
