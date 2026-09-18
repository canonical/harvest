import { ref } from 'vue';

export function useStickyScroll(threshold = 24) {
  const stuck = ref(true);

  function handleScroll(el) {
    if (!el) return;
    stuck.value = el.scrollHeight - el.scrollTop - el.clientHeight <= threshold;
  }

  function follow(el) {
    if (!el || !stuck.value) return;
    el.scrollTop = el.scrollHeight;
  }

  function jumpToLatest(el) {
    stuck.value = true;
    if (el) el.scrollTop = el.scrollHeight;
  }

  return { stuck, handleScroll, follow, jumpToLatest };
}
