import type { ReactNode } from "react";

/** Centered card layout shared by the setup + login screens. */
export function AuthScreen({
  title,
  subtitle,
  children,
  footer,
}: {
  title: string;
  subtitle: string;
  children: ReactNode;
  footer?: ReactNode;
}) {
  return (
    <div className="flex min-h-[100dvh] items-center justify-center bg-ink-950 px-4 py-8 pb-[max(2rem,env(safe-area-inset-bottom))] pt-[max(2rem,env(safe-area-inset-top))]">
      <div className="w-full max-w-sm">
        <div className="mb-8 text-center">
          <div className="mx-auto mb-4 flex h-12 w-12 items-center justify-center rounded-xl bg-accent/10 ring-1 ring-accent/30">
            <span className="text-xl font-bold text-accent-soft">c</span>
          </div>
          <h1 className="text-xl font-semibold text-ink-100">{title}</h1>
          <p className="mt-1.5 text-sm text-ink-500">{subtitle}</p>
        </div>
        <div className="rounded-2xl border border-ink-700 bg-ink-900/50 p-5 shadow-xl shadow-black/40 sm:p-6">
          {children}
        </div>
        {footer && <div className="mt-4 text-center text-xs text-ink-600">{footer}</div>}
      </div>
    </div>
  );
}
