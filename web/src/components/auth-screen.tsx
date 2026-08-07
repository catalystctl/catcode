import type { ReactNode } from "react";
import { BrandMark } from "@/components/icons";

/** Compact workbench identity shared by the setup + login screens. */
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
    <main className="flex min-h-[100dvh] items-center justify-center bg-ink-950 px-5 pb-[max(2rem,env(safe-area-inset-bottom))] pt-[max(2rem,env(safe-area-inset-top))]">
      <section className="w-full max-w-sm border-l border-ink-800 pl-5 sm:pl-6" aria-labelledby="auth-title">
        <div className="mb-7">
          <div className="flex items-center gap-2.5">
            <BrandMark size={24} />
            <span className="font-display text-[13px] font-semibold text-ink-200">CatCode</span>
          </div>
          <p className="mt-5 font-mono text-[10px] uppercase tracking-[0.14em] text-ink-500">
            Workbench access
          </p>
          <h1 id="auth-title" className="mt-1.5 font-display text-xl font-semibold text-ink-100">
            {title}
          </h1>
          <p className="mt-1.5 text-[13px] leading-relaxed text-ink-500">{subtitle}</p>
        </div>
        {children}
        {footer && <div className="mt-5 font-mono text-[10px] text-ink-600">{footer}</div>}
      </section>
    </main>
  );
}
