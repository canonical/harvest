import { FitAddon } from '@xterm/addon-fit';
import { SearchAddon } from '@xterm/addon-search';
import { Unicode11Addon } from '@xterm/addon-unicode11';
import { WebLinksAddon } from '@xterm/addon-web-links';
import { ClipboardAddon } from '@xterm/addon-clipboard';
import { WebglAddon } from '@xterm/addon-webgl';

export function openLink(_event, uri) {
  window.open(uri, '_blank', 'noopener,noreferrer');
}

export function loadAddons(terminal) {
  const fit = new FitAddon();
  const search = new SearchAddon();
  terminal.loadAddon(fit);
  terminal.loadAddon(search);
  terminal.loadAddon(new Unicode11Addon());
  terminal.unicode.activeVersion = '11';
  terminal.loadAddon(new WebLinksAddon(openLink));
  terminal.loadAddon(new ClipboardAddon());
  return { fit, search };
}

export function loadRenderer(terminal) {
  try {
    const webgl = new WebglAddon();
    webgl.onContextLoss(() => webgl.dispose());
    terminal.loadAddon(webgl);
    return 'webgl';
  } catch {
    return 'dom';
  }
}
