// Shared, framework-agnostic helpers for mapping file paths to editor language
// ids and extracting basenames. Deliberately plain (no "use client", no React)
// so it is safe to import from both client components and Node route handlers.
// Used by use-ide.ts (openFile) and api/file/route.ts (GET language field).

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
    default:
      return undefined;
  }
}

/** Basename of a workspace-relative or absolute path (forward or back slashes). */
export function basename(path: string): string {
  const parts = path.replace(/\\/g, "/").split("/").filter(Boolean);
  return parts[parts.length - 1] ?? path;
}
