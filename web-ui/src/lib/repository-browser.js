export const ROOT = '';

const IMAGE_EXTENSIONS = new Set(['png', 'jpg', 'jpeg', 'gif', 'webp', 'avif', 'bmp', 'ico', 'svg']);
const MARKDOWN_EXTENSIONS = new Set(['md', 'markdown', 'mdown', 'mkd']);
const README_NAME = /^readme(\.[a-z]+)?$/i;
const SIZE_UNITS = ['bytes', 'KB', 'MB', 'GB'];

export function nameOf(path) {
  return path.split('/').pop() ?? '';
}

export function extensionOf(path) {
  const name = nameOf(path);
  const dot = name.lastIndexOf('.');
  return dot > 0 ? name.slice(dot + 1).toLowerCase() : '';
}

export function parentOf(path) {
  const slash = path.lastIndexOf('/');
  return slash < 0 ? ROOT : path.slice(0, slash);
}

export function breadcrumbs(path) {
  const segments = path.split('/').filter(Boolean);
  return segments.map((name, index) => ({ name, path: segments.slice(0, index + 1).join('/') }));
}

export function ancestorsOf(path) {
  return breadcrumbs(path).slice(0, -1).map((crumb) => crumb.path);
}

export const isDirectory = (entry) => entry.kind === 'directory';
export const isImage = (path) => IMAGE_EXTENSIONS.has(extensionOf(path));
export const isMarkdown = (path) => MARKDOWN_EXTENSIONS.has(extensionOf(path));

export function viewerFor(file) {
  if (isImage(file.path)) return 'image';
  if (file.kind === 'text') return isMarkdown(file.path) ? 'markdown' : 'code';
  return file.kind === 'too_large' ? 'too-large' : 'binary';
}

export function findReadme(entries) {
  const readmes = entries.filter((entry) => !isDirectory(entry) && README_NAME.test(entry.name));
  return readmes.find((entry) => isMarkdown(entry.path)) ?? readmes[0] ?? null;
}

export function formatSize(bytes) {
  let size = bytes;
  let unit = 0;
  while (size >= 1024 && unit < SIZE_UNITS.length - 1) {
    size /= 1024;
    unit += 1;
  }
  const rounded = unit === 0 ? String(size) : size.toFixed(1).replace(/\.0$/, '');
  return `${rounded} ${SIZE_UNITS[unit]}`;
}

const textParam = (value) => (typeof value === 'string' ? value : '');

function lineParam(value) {
  const line = Number.parseInt(textParam(value), 10);
  return line > 0 ? line : null;
}

export function locationFromQuery(query) {
  const file = textParam(query.file);
  return {
    version: textParam(query.version) || null,
    path: file || textParam(query.dir),
    isFile: Boolean(file),
    line: file ? lineParam(query.line) : null,
  };
}

export function queryFromLocation({ version, path, isFile, line }) {
  return {
    ...(version ? { version } : {}),
    ...(path ? { [isFile ? 'file' : 'dir']: path } : {}),
    ...(isFile && line ? { line: String(line) } : {}),
  };
}

export function visibleRows(childrenByPath, expanded, directory = ROOT, depth = 0) {
  const rows = [];
  for (const entry of childrenByPath.get(directory) ?? []) {
    const isOpen = isDirectory(entry) && expanded.has(entry.path);
    rows.push({ entry, depth, isOpen, isLoading: isOpen && !childrenByPath.has(entry.path) });
    if (isOpen) rows.push(...visibleRows(childrenByPath, expanded, entry.path, depth + 1));
  }
  return rows;
}

export function cachedLoader(load) {
  const cache = new Map();
  return (path) => {
    if (!cache.has(path)) {
      const pending = load(path).catch((error) => {
        cache.delete(path);
        throw error;
      });
      cache.set(path, pending);
    }
    return cache.get(path);
  };
}
