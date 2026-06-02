import cytoscape from 'cytoscape';
import fcose from 'cytoscape-fcose';

cytoscape.use(fcose);

export const KIND_COLORS = {
  class:     '#7C3AED',
  struct:    '#6D28D9',
  trait:     '#0891B2',
  interface: '#0284C7',
  function:  '#2563EB',
  method:    '#1D4ED8',
  enum:      '#D97706',
  module:    '#6B7280',
  impl:      '#059669',
  type:      '#DB2777',
};

export function kindColor(kind) {
  return KIND_COLORS[kind?.toLowerCase()] ?? '#4F46E5';
}

export function kindShape(kind) {
  const k = kind?.toLowerCase() ?? '';
  if (k === 'class' || k === 'struct' || k === 'trait' || k === 'interface' || k === 'enum') {
    return 'rectangle';
  }
  return 'round-rectangle';
}

export function cytoscapeStyle() {
  return [
    {
      selector: 'node.file-node',
      style: {
        label: 'data(label)',
        'background-color': 'data(color)',
        'background-opacity': 0.06,
        'border-width': 1.5,
        'border-color': 'data(color)',
        'border-opacity': 0.55,
        color: 'data(color)',
        'font-size': '11px',
        'font-family': '"Ubuntu Mono", "Courier New", monospace',
        'font-weight': '600',
        'text-valign': 'top',
        'text-halign': 'center',
        'text-margin-y': -6,
        padding: '24px 16px 14px',
        'min-width': 60,
        'min-height': 32,
        'transition-property': 'opacity, border-opacity, background-opacity',
        'transition-duration': '180ms',
        cursor: 'default',
      },
    },
    { selector: 'node.file-node.dimmed', style: { opacity: 0.1 } },
    {
      selector: 'node.file-node.ring',
      style: { 'border-opacity': 1, 'border-width': 2.5, 'background-opacity': 0.1 },
    },
    {
      selector: 'node.symbol-node',
      style: {
        label: 'data(label)',
        'background-color': 'data(color)',
        color: '#ffffff',
        'font-size': '11px',
        'font-family': '"Ubuntu Mono", "Courier New", monospace',
        'text-valign': 'center',
        'text-halign': 'center',
        'text-wrap': 'ellipsis',
        'text-max-width': '110px',
        shape: 'data(shape)',
        width: 'label',
        height: 'label',
        padding: '6px 11px',
        'min-width': 48,
        'min-height': 22,
        'border-width': 0,
        'border-color': '#ffffff',
        'transition-property': 'opacity, border-width',
        'transition-duration': '150ms',
        cursor: 'pointer',
      },
    },
    { selector: 'node.symbol-node:selected', style: { 'border-width': 3, 'border-color': '#ffffff' } },
    { selector: 'node.symbol-node.dimmed', style: { opacity: 0.1, color: 'transparent' } },
    { selector: 'node.symbol-node.ring', style: { 'border-width': 3, 'border-color': '#FFD700' } },
    {
      selector: 'edge',
      style: {
        'curve-style': 'bezier',
        'target-arrow-shape': 'none',
        'source-arrow-shape': 'none',
        'arrow-scale': 0.9,
        width: 1.5,
        'transition-property': 'opacity, line-color, target-arrow-color, source-arrow-color, width',
        'transition-duration': '150ms',
      },
    },
    {
      selector: 'edge[relation="inherits"]',
      style: {
        'line-color': 'rgba(124,58,237,0.85)',
        'target-arrow-shape': 'triangle',
        'target-arrow-fill': 'hollow',
        'target-arrow-color': 'rgba(124,58,237,0.85)',
        'line-style': 'solid',
        width: 2,
      },
    },
    {
      selector: 'edge[relation="implements"]',
      style: {
        'line-color': 'rgba(8,145,178,0.85)',
        'target-arrow-shape': 'triangle',
        'target-arrow-fill': 'hollow',
        'target-arrow-color': 'rgba(8,145,178,0.85)',
        'line-style': 'dashed',
        'line-dash-pattern': [6, 3],
        width: 2,
      },
    },
    {
      selector: 'edge[relation="embeds"]',
      style: {
        'line-color': 'rgba(5,150,105,0.7)',
        'source-arrow-shape': 'diamond',
        'source-arrow-fill': 'hollow',
        'source-arrow-color': 'rgba(5,150,105,0.85)',
        'line-style': 'solid',
        width: 1.5,
      },
    },
    {
      selector: 'edge[relation="contains"]',
      style: {
        'line-color': 'rgba(139,92,246,0.55)',
        'source-arrow-shape': 'diamond',
        'source-arrow-fill': 'filled',
        'source-arrow-color': 'rgba(139,92,246,0.75)',
        'line-style': 'solid',
        width: 1.5,
      },
    },
    {
      selector: 'edge[relation="uses"]',
      style: {
        'line-color': 'rgba(217,119,6,0.6)',
        'target-arrow-shape': 'triangle',
        'target-arrow-color': 'rgba(217,119,6,0.75)',
        'line-style': 'solid',
        width: 1,
      },
    },
    {
      selector: 'edge[relation="calls"]',
      style: {
        'line-color': 'rgba(120,120,120,0.4)',
        'target-arrow-shape': 'triangle',
        'target-arrow-color': 'rgba(120,120,120,0.55)',
        'line-style': 'dashed',
        'line-dash-pattern': [5, 3],
      },
    },
    { selector: 'edge.dimmed', style: { opacity: 0.04 } },
    {
      selector: 'edge[relation="inherits"].active',
      style: { 'line-color': '#7C3AED', 'target-arrow-color': '#7C3AED', width: 2.5, opacity: 1 },
    },
    {
      selector: 'edge[relation="implements"].active',
      style: { 'line-color': '#0891B2', 'target-arrow-color': '#0891B2', width: 2.5, opacity: 1 },
    },
    {
      selector: 'edge[relation="embeds"].active',
      style: { 'line-color': '#059669', 'source-arrow-color': '#059669', width: 2, opacity: 1 },
    },
    {
      selector: 'edge[relation="uses"].active',
      style: { 'line-color': '#D97706', 'target-arrow-color': '#D97706', width: 2, opacity: 1 },
    },
    {
      selector: 'edge[relation="contains"].active',
      style: { 'line-color': '#8B5CF6', 'source-arrow-color': '#8B5CF6', width: 2, opacity: 1 },
    },
    {
      selector: 'edge[relation="calls"].active',
      style: { 'line-color': '#3B82F6', 'target-arrow-color': '#3B82F6', width: 2.5, opacity: 1 },
    },
  ];
}
