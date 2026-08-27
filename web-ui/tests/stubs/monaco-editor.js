export const editor = {
  create() {
    return {
      getValue() { return ''; },
      setValue() {},
      onDidChangeModelContent() {},
      dispose() {},
      updateOptions() {},
    };
  },
};
export default { editor };
