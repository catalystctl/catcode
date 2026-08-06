// / — the project-centric terminal workspace (hub).
// Auth-gated: no account → setup, no session → login. The shell is
// client-rendered (project tabs + terminal grid + git sidebar).

import { headers } from "next/headers";
import { redirect } from "next/navigation";
import { ErrorBoundary } from "@/components/error-boundary";
import { HubShell } from "@/components/hub/hub-shell";
import { accountExists, getSession } from "@/lib/auth";

export const metadata = { title: "Catalyst Code · Hub" };

export default async function Page() {
  const h = await headers();
  if (!(await accountExists())) redirect("/setup");
  const session = await getSession(h);
  if (!session) redirect("/login");

  return (
    <ErrorBoundary label="hub">
      <HubShell />
    </ErrorBoundary>
  );
}
