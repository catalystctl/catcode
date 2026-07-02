"use client";

// ImageAttach — image attachment affordance for the composer. A paperclip
// button opens a file picker; selected images are read as data URLs (the
// shape the core's `send` command expects) and surfaced as a thumbnail strip
// with per-image remove. `fileToDataUrl` is exported so the composer can reuse
// it for clipboard-paste handling.

import { forwardRef, useImperativeHandle, useRef, useState } from "react";
import { XIcon } from "./icons";

/** Read a File into a data URL string (FileReader.readAsDataURL). */
export function fileToDataUrl(file: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(String(reader.result ?? ""));
    reader.onerror = () => reject(reader.error ?? new Error("read failed"));
    reader.readAsDataURL(file);
  });
}

interface Props {
  images: string[];
  onAdd: (dataUrl: string) => void;
  onRemove: (i: number) => void;
}

/** Imperative handle so the Composer (and /attach command) can open the picker. */
export interface ImageAttachHandle {
  pick: () => void;
}

export const ImageAttach = forwardRef<ImageAttachHandle, Props>(function ImageAttach(
  { images, onAdd, onRemove },
  ref,
) {
  const inputRef = useRef<HTMLInputElement>(null);
  const [busy, setBusy] = useState(false);

  const pick = () => inputRef.current?.click();

  useImperativeHandle(ref, () => ({ pick }), []);

  const onChange = async (e: React.ChangeEvent<HTMLInputElement>) => {
    const files = Array.from(e.target.files ?? []);
    e.target.value = "";
    if (files.length === 0) return;
    setBusy(true);
    try {
      for (const f of files) {
        const url = await fileToDataUrl(f);
        onAdd(url);
      }
    } finally {
      setBusy(false);
    }
  };

  if (images.length === 0) {
    return (
      <>
        <input
          ref={inputRef}
          type="file"
          accept="image/*"
          multiple
          className="hidden"
          onChange={onChange}
        />
        <button
          type="button"
          onClick={pick}
          disabled={busy}
          aria-label="Attach image"
          title="Attach image"
          className="flex h-9 w-9 shrink-0 items-center justify-center rounded-xl text-ink-400 transition-colors hover:bg-ink-850 hover:text-ink-100 disabled:opacity-50"
        >
          <svg width={16} height={16} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={2} strokeLinecap="round" strokeLinejoin="round">
            <path d="M21.44 11.05l-9.19 9.19a6 6 0 0 1-8.49-8.49l9.19-9.19a4 4 0 0 1 5.66 5.66l-9.2 9.19a2 2 0 0 1-2.83-2.83l8.49-8.48" />
          </svg>
        </button>
      </>
    );
  }

  return (
    <div className="flex flex-wrap items-center gap-1.5">
      {images.map((src, i) => (
        <div key={i} className="group/img relative h-12 w-12">
          {/* eslint-disable-next-line @next/next/no-img-element */}
          <img
            src={src}
            alt={`attachment ${i + 1}`}
            className="h-12 w-12 rounded-md border border-ink-700 object-cover"
          />
          <button
            type="button"
            onClick={() => onRemove(i)}
            aria-label={`Remove attachment ${i + 1}`}
            className="absolute -right-1.5 -top-1.5 flex h-4 w-4 items-center justify-center rounded-full border border-ink-700 bg-ink-950 text-ink-300 opacity-0 transition-opacity hover:bg-rose-500/80 hover:text-white group-hover/img:opacity-100"
          >
            <XIcon width={10} height={10} />
          </button>
        </div>
      ))}
      <input
        ref={inputRef}
        type="file"
        accept="image/*"
        multiple
        className="hidden"
        onChange={onChange}
      />
      <button
        type="button"
        onClick={pick}
        disabled={busy}
        aria-label="Attach image"
        title="Attach image"
        className="flex h-12 w-12 shrink-0 items-center justify-center rounded-md border border-dashed border-ink-700 text-ink-400 transition-colors hover:border-ink-600 hover:text-ink-100 disabled:opacity-50"
      >
        <svg width={16} height={16} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={2} strokeLinecap="round" strokeLinejoin="round">
          <path d="M12 5v14M5 12h14" />
        </svg>
      </button>
    </div>
  );
});
