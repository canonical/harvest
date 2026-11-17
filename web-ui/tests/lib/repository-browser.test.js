import { describe, it, expect, vi } from 'vitest';
import {
  nameOf,
  extensionOf,
  parentOf,
  breadcrumbs,
  ancestorsOf,
  viewerFor,
  findReadme,
  formatSize,
  locationFromQuery,
  queryFromLocation,
  visibleRows,
  cachedLoader,
} from '../../src/lib/repository-browser.js';

const file = (path) => ({ name: nameOf(path), path, kind: 'file' });
const directory = (path) => ({ name: nameOf(path), path, kind: 'directory' });

describe('repository paths', () => {
  it('splits a path into its name, extension and parent', () => {
    expect(nameOf('src/lib/main.rs')).toBe('main.rs');
    expect(extensionOf('docs/Guide.MD')).toBe('md');
    expect(extensionOf('Makefile')).toBe('');
    expect(extensionOf('.gitignore')).toBe('');
    expect(parentOf('src/lib/main.rs')).toBe('src/lib');
    expect(parentOf('README.md')).toBe('');
  });

  it('builds breadcrumbs and ancestors from the root down', () => {
    expect(breadcrumbs('src/lib/main.rs')).toEqual([
      { name: 'src', path: 'src' },
      { name: 'lib', path: 'src/lib' },
      { name: 'main.rs', path: 'src/lib/main.rs' },
    ]);
    expect(ancestorsOf('src/lib/main.rs')).toEqual(['src', 'src/lib']);
    expect(ancestorsOf('README.md')).toEqual([]);
    expect(breadcrumbs('')).toEqual([]);
  });
});

describe('file viewers', () => {
  it('shows images, markdown, code, binaries and oversized files differently', () => {
    expect(viewerFor({ path: 'logo.svg', kind: 'text' })).toBe('image');
    expect(viewerFor({ path: 'photo.PNG', kind: 'binary' })).toBe('image');
    expect(viewerFor({ path: 'README.md', kind: 'text' })).toBe('markdown');
    expect(viewerFor({ path: 'main.rs', kind: 'text' })).toBe('code');
    expect(viewerFor({ path: 'app.bin', kind: 'binary' })).toBe('binary');
    expect(viewerFor({ path: 'dump.sql', kind: 'too_large' })).toBe('too-large');
  });

  it('prefers a markdown readme and ignores directories named readme', () => {
    expect(findReadme([directory('readme'), file('README.txt'), file('README.md')]).path).toBe('README.md');
    expect(findReadme([file('readme')]).path).toBe('readme');
    expect(findReadme([file('main.rs')])).toBeNull();
  });

  it('formats sizes in the largest whole unit', () => {
    expect(formatSize(512)).toBe('512 bytes');
    expect(formatSize(2048)).toBe('2 KB');
    expect(formatSize(1536)).toBe('1.5 KB');
    expect(formatSize(5 * 1024 * 1024)).toBe('5 MB');
  });
});

describe('browser location', () => {
  it('reads a file, its line and the version from the query', () => {
    expect(locationFromQuery({ version: 'main', file: 'src/a.rs', line: '42' })).toEqual({
      version: 'main', path: 'src/a.rs', isFile: true, line: 42,
    });
  });

  it('reads a directory and ignores lines outside files', () => {
    expect(locationFromQuery({ dir: 'src', line: '3' })).toEqual({ version: null, path: 'src', isFile: false, line: null });
    expect(locationFromQuery({})).toEqual({ version: null, path: '', isFile: false, line: null });
    expect(locationFromQuery({ file: 'a.rs', line: 'x' }).line).toBeNull();
  });

  it('round-trips through the query and leaves the root out', () => {
    const location = { version: 'v2', path: 'src/a.rs', isFile: true, line: 7 };
    expect(queryFromLocation(location)).toEqual({ version: 'v2', file: 'src/a.rs', line: '7' });
    expect(locationFromQuery(queryFromLocation(location))).toEqual(location);
    expect(queryFromLocation({ version: null, path: '', isFile: false, line: null })).toEqual({});
  });
});

describe('visible tree rows', () => {
  const children = new Map([
    ['', [directory('src'), file('README.md')]],
    ['src', [directory('src/lib'), file('src/main.rs')]],
  ]);

  it('shows only the top level when nothing is expanded', () => {
    expect(visibleRows(children, new Set()).map((row) => row.entry.path)).toEqual(['src', 'README.md']);
  });

  it('nests the children of expanded directories one level deeper', () => {
    const rows = visibleRows(children, new Set(['src']));
    expect(rows.map((row) => [row.entry.path, row.depth])).toEqual([
      ['src', 0], ['src/lib', 1], ['src/main.rs', 1], ['README.md', 0],
    ]);
    expect(rows[0].isOpen).toBe(true);
  });

  it('marks an expanded directory whose children have not arrived yet', () => {
    const rows = visibleRows(children, new Set(['src', 'src/lib']));
    expect(rows.find((row) => row.entry.path === 'src/lib').isLoading).toBe(true);
    expect(rows.find((row) => row.entry.path === 'src').isLoading).toBe(false);
  });
});

describe('cached loader', () => {
  it('loads each path once', async () => {
    const load = vi.fn(async (path) => [path]);
    const children = cachedLoader(load);
    await children('src');
    await children('src');
    expect(load).toHaveBeenCalledTimes(1);
  });

  it('tries again after a failure', async () => {
    const load = vi.fn().mockRejectedValueOnce(new Error('offline')).mockResolvedValue(['ok']);
    const children = cachedLoader(load);
    await expect(children('src')).rejects.toThrow('offline');
    await expect(children('src')).resolves.toEqual(['ok']);
  });
});
