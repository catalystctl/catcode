/** File types rendered by the in-app Preview panel. Keep this separate from
 * the preview component so the editor can check support without pulling the
 * Markdown renderer into its lazy chunk. */
export const PREVIEW_IMAGE_EXTENSIONS = new Set([
  "svg",
  "png",
  "jpg",
  "jpeg",
  "gif",
  "webp",
]);

export function previewExtension(path: string): string {
  const clean = path.split(/[?#]/, 1)[0];
  const filename = clean.replace(/\\/g, "/").split("/").pop() ?? clean;
  const dot = filename.lastIndexOf(".");
  return dot < 0 ? "" : filename.slice(dot + 1).toLowerCase();
}

export function canPreviewFile(path: string): boolean {
  const extension = previewExtension(path);
  return (
    extension === "md" ||
    extension === "markdown" ||
    extension === "html" ||
    extension === "htm" ||
    extension === "pdf" ||
    PREVIEW_IMAGE_EXTENSIONS.has(extension)
  );
}
