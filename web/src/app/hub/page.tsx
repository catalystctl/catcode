// /hub — the project-centric terminal workspace (alternative to the IDE at /).
// Auth-gated exactly like the main page: no account → setup, no session →
// login. The shell itself is client-rendered (project tabs + terminal grid +
// git sidebar); the server render is just a hydration splash.

import { headers } from "next/headers";
import { redirect } from "next/navigation";
import { ErrorBoundary } from "@/components/error-boundary";
import { HubShell } from "@/components/hub/hub-shell";
import { accountExists, getSession } from "@/lib/auth";

export const metadata = { title: "Catalyst Code · Hub" };

export default async function HubPage() {
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
