// Shared, framework-agnostic helpers for mapping file paths to editor language
// ids. Safe to import from both client components and Node route handlers.

/** Detect an editor language id from a file path's extension. */
export function detectLanguage(path: string): string | undefined {
  const ext = path.split(".").pop()?.toLowerCase();
  switch (ext) {
    case "ts":
    case "tsx":
      return "typescript";
    case "js":
    case "mjs":
    case "cjs":
    case "jsx":
      return "javascript";
    case "py":
    case "pyw":
      return "python";
    case "rs":
      return "rust";
    case "md":
    case "markdown":
      return "markdown";
    case "json":
      return "json";
    case "css":
    case "scss":
    case "sass":
    case "less":
      return "css";
    case "html":
    case "htm":
      return "html";
    case "yml":
    case "yaml":
      return "yaml";
    case "sql":
      return "sql";
    case "go":
      return "go";
    case "sh":
    case "bash":
    case "zsh":
      return "shell";
    case "toml":
      return "toml";
    case "xml":
      return "xml";
    case "svg":
      return "xml";
    case "txt":
    case "log":
      return "plaintext";
    default:
      return undefined;
  }
}

/** Just the basename of a path (forward- or backslash-separated). */
export function basename(path: string): string {
  const parts = path.replace(/\\/g, "/").split("/");
  return parts[parts.length - 1] || path;
}
