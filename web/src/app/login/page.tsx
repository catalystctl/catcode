import { headers } from "next/headers";
import { redirect } from "next/navigation";
import { accountExists, getSession } from "@/lib/auth";
import { LoginForm } from "@/components/login";

export const dynamic = "force-dynamic";

export default async function LoginPage() {
  // No account yet → first-run setup. Already authed → the app.
  if (!(await accountExists())) redirect("/setup");
  const h = await headers();
  const session = await getSession(h);
  if (session) redirect("/");

  return <LoginForm />;
}
