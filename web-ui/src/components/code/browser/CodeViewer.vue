<template>
  <div ref="container" class="code-viewer" :aria-label="`Contents of ${path}`" />
</template>

<script setup>
import { onMounted, onUnmounted, ref, watch } from 'vue';
import { languageForPath, loadMonaco, monacoTheme } from '../../../lib/monaco.js';
import { onThemeChange } from '../../../lib/theme.js';

const props = defineProps({
  path: { type: String, required: true },
  content: { type: String, required: true },
  line: { type: Number, default: null },
});
const emit = defineEmits(['select-line']);

const container = ref(null);
let monaco = null;
let editor = null;
let lineHighlight = null;

function showContent() {
  const language = languageForPath(monaco.languages.getLanguages(), props.path);
  const previous = editor.getModel();
  editor.setModel(monaco.editor.createModel(props.content, language));
  previous?.dispose();
}

function highlightLine() {
  lineHighlight?.clear();
  if (!props.line) return;
  lineHighlight = editor.createDecorationsCollection([
    { range: new monaco.Range(props.line, 1, props.line, 1), options: { isWholeLine: true, className: 'code-viewer__line' } },
  ]);
  editor.revealLineInCenter(props.line);
}

function reportLineNumberClicks() {
  editor.onMouseDown((event) => {
    if (event.target.type === monaco.editor.MouseTargetType.GUTTER_LINE_NUMBERS) {
      emit('select-line', event.target.position.lineNumber);
    }
  });
}

const stopFollowingTheme = onThemeChange(() => monaco?.editor.setTheme(monacoTheme()));

onMounted(async () => {
  monaco = await loadMonaco();
  if (!container.value) return;
  editor = monaco.editor.create(container.value, {
    readOnly: true,
    domReadOnly: true,
    theme: monacoTheme(),
    automaticLayout: true,
    minimap: { enabled: false },
    scrollBeyondLastLine: false,
    renderLineHighlight: 'none',
    contextmenu: false,
    occurrencesHighlight: 'off',
    selectionHighlight: false,
    links: false,
    codeLens: false,
    colorDecorators: false,
  });
  showContent();
  highlightLine();
  reportLineNumberClicks();
});

watch(() => [props.path, props.content], () => {
  if (!editor) return;
  showContent();
  highlightLine();
});

watch(() => props.line, () => editor && highlightLine());

onUnmounted(() => {
  stopFollowingTheme();
  const model = editor?.getModel();
  editor?.dispose();
  model?.dispose();
});
</script>
