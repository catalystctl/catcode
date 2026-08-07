// /hub — permanent alias of the hub frontend (now also at /).
// Kept so bookmarks, update-web.sh health checks, and e2e scripts that hit
// /hub keep working without a breaking URL change.

import { headers } from "next/headers";
import { redirect } from "next/navigation";
import { ErrorBoundary } from "@/components/error-boundary";
import { HubShell } from "@/components/hub/hub-shell";
import { accountExists, getSession } from "@/lib/auth";

export const metadata = { title: "Catalyst Code" };

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
