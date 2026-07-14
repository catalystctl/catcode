import type * as Monaco from "monaco-editor";

type MonacoApi = typeof Monaco;

let monacoPromise: Promise<MonacoApi> | null = null;

function workerFor(label: string): Worker {
  const options: WorkerOptions = { type: "module", name: `monaco-${label}` };
  if (label === "json") {
    return new Worker(new URL("monaco-editor/esm/vs/language/json/json.worker.js", import.meta.url), options);
  }
  if (label === "css" || label === "scss" || label === "less") {
    return new Worker(new URL("monaco-editor/esm/vs/language/css/css.worker.js", import.meta.url), options);
  }
  if (label === "html" || label === "handlebars" || label === "razor") {
    return new Worker(new URL("monaco-editor/esm/vs/language/html/html.worker.js", import.meta.url), options);
  }
  if (label === "typescript" || label === "javascript") {
    return new Worker(new URL("monaco-editor/esm/vs/language/typescript/ts.worker.js", import.meta.url), options);
  }
  return new Worker(new URL("monaco-editor/esm/vs/editor/editor.worker.js", import.meta.url), options);
}

function defineCatalystThemes(monaco: MonacoApi): void {
  monaco.editor.defineTheme("catalyst-dark", {
    base: "vs-dark",
    inherit: true,
    rules: [
      { token: "comment", foreground: "757575", fontStyle: "italic" },
      { token: "keyword", foreground: "DE8FA2" },
      { token: "number", foreground: "D9B48F" },
      { token: "string", foreground: "A7C080" },
      { token: "type", foreground: "7FB4CA" },
      { token: "type.identifier", foreground: "7FB4CA" },
      { token: "identifier", foreground: "F0F0F0" },
      { token: "delimiter", foreground: "A3A3A3" },
      { token: "tag", foreground: "CF8A59" },
      { token: "attribute.name", foreground: "D9B48F" },
    ],
    colors: {
      "editor.background": "#1A1A1A",
      "editor.foreground": "#F0F0F0",
      "editorGutter.background": "#1A1A1A",
      "editorLineNumber.foreground": "#5F5F5F",
      "editorLineNumber.activeForeground": "#D4D4D4",
      "editorCursor.foreground": "#DEA57C",
      "editor.selectionBackground": "#CF8A5955",
      "editor.inactiveSelectionBackground": "#CF8A5933",
      "editor.selectionHighlightBackground": "#CF8A5926",
      "editor.lineHighlightBackground": "#242424",
      "editor.lineHighlightBorder": "#00000000",
      "editorIndentGuide.background1": "#383838",
      "editorIndentGuide.activeBackground1": "#757575",
      "editorWhitespace.foreground": "#383838",
      "editorBracketMatch.background": "#CF8A5933",
      "editorBracketMatch.border": "#CF8A59",
      "editor.findMatchBackground": "#F59F0A66",
      "editor.findMatchHighlightBackground": "#F59F0A33",
      "editorWidget.background": "#242424",
      "editorWidget.border": "#383838",
      "editorHoverWidget.background": "#242424",
      "editorHoverWidget.border": "#383838",
      "editorSuggestWidget.background": "#242424",
      "editorSuggestWidget.border": "#383838",
      "editorSuggestWidget.selectedBackground": "#333333",
      "input.background": "#1F1F1F",
      "input.border": "#383838",
      "focusBorder": "#CF8A59",
      "scrollbarSlider.background": "#38383888",
      "scrollbarSlider.hoverBackground": "#525252AA",
      "scrollbarSlider.activeBackground": "#757575AA",
      "minimap.background": "#1A1A1A",
    },
  });

  monaco.editor.defineTheme("catalyst-light", {
    base: "vs",
    inherit: true,
    rules: [
      { token: "comment", foreground: "666666", fontStyle: "italic" },
      { token: "keyword", foreground: "A3154F" },
      { token: "number", foreground: "986801" },
      { token: "string", foreground: "397300" },
      { token: "type", foreground: "0E7490" },
      { token: "type.identifier", foreground: "0E7490" },
      { token: "identifier", foreground: "141414" },
      { token: "delimiter", foreground: "666666" },
      { token: "tag", foreground: "B46E3C" },
      { token: "attribute.name", foreground: "986801" },
    ],
    colors: {
      "editor.background": "#FAFAFA",
      "editor.foreground": "#141414",
      "editorGutter.background": "#FAFAFA",
      "editorLineNumber.foreground": "#808080",
      "editorLineNumber.activeForeground": "#333333",
      "editorCursor.foreground": "#B46E3C",
      "editor.selectionBackground": "#C8825055",
      "editor.inactiveSelectionBackground": "#C8825033",
      "editor.selectionHighlightBackground": "#C8825026",
      "editor.lineHighlightBackground": "#F0F0F0",
      "editor.lineHighlightBorder": "#00000000",
      "editorIndentGuide.background1": "#D6D6D6",
      "editorIndentGuide.activeBackground1": "#999999",
      "editorWhitespace.foreground": "#CCCCCC",
      "editorBracketMatch.background": "#B46E3C22",
      "editorBracketMatch.border": "#B46E3C",
      "editor.findMatchBackground": "#F59F0A66",
      "editor.findMatchHighlightBackground": "#F59F0A33",
      "editorWidget.background": "#FFFFFF",
      "editorWidget.border": "#CCCCCC",
      "editorHoverWidget.background": "#FFFFFF",
      "editorHoverWidget.border": "#CCCCCC",
      "editorSuggestWidget.background": "#FFFFFF",
      "editorSuggestWidget.border": "#CCCCCC",
      "editorSuggestWidget.selectedBackground": "#F0F0F0",
      "input.background": "#FFFFFF",
      "input.border": "#CCCCCC",
      "focusBorder": "#B46E3C",
      "scrollbarSlider.background": "#CCCCCC88",
      "scrollbarSlider.hoverBackground": "#9A9A9AAA",
      "scrollbarSlider.activeBackground": "#808080AA",
      "minimap.background": "#FAFAFA",
    },
  });
}

export function loadMonaco(): Promise<MonacoApi> {
  if (!monacoPromise) {
    globalThis.MonacoEnvironment = { getWorker: (_moduleId, label) => workerFor(label) };
    monacoPromise = import("monaco-editor").then((monaco) => {
      defineCatalystThemes(monaco);
      return monaco;
    });
  }
  return monacoPromise;
}

export function currentMonacoTheme(): "catalyst-dark" | "catalyst-light" {
  return document.documentElement.dataset.theme === "light" ? "catalyst-light" : "catalyst-dark";
}
