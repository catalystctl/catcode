import { headers } from "next/headers";
import { redirect } from "next/navigation";
import { Chat } from "@/components/chat";
import { ErrorBoundary } from "@/components/error-boundary";
import { accountExists, getSession } from "@/lib/auth";

export default async function Page() {
  const h = await headers();
  // No account yet → first-run setup. No session → login. Else → the app.
  if (!(await accountExists())) redirect("/setup");
  const session = await getSession(h);
  if (!session) redirect("/login");

  return (
    <ErrorBoundary label="app">
      <Chat />
    </ErrorBoundary>
  );
}
