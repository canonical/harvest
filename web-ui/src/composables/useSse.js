export function useSse() {
  let aborted = false;
  let readerRef = null;

  function abort() {
    aborted = true;
    readerRef?.cancel().catch(() => {});
  }

  async function stream(url, options, onEvent) {
    aborted = false;
    const res = await fetch(url, options);
    if (!res.ok) throw new Error(`SSE request failed: ${res.status}`);

    const reader = res.body.getReader();
    readerRef = reader;
    const decoder = new TextDecoder();
    let buffer = '';

    try {
      while (true) {
        if (aborted) break;
        const { done, value } = await reader.read();
        if (done) break;
        buffer += decoder.decode(value, { stream: true });

        const lines = buffer.split('\n');
        buffer = lines.pop() ?? '';

        for (const line of lines) {
          if (!line.startsWith('data:')) continue;
          const json = line.slice(5).trim();
          if (!json) continue;
          try {
            onEvent(JSON.parse(json));
          } catch {
          }
        }
      }
    } finally {
      reader.releaseLock();
    }
  }

  return { stream, abort };
}
