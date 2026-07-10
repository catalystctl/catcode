import { headers } from "next/headers";
import { redirect } from "next/navigation";
import { accountExists, getSession } from "@/lib/auth";
import { SetupForm } from "@/components/setup";

export const dynamic = "force-dynamic";

export default async function SetupPage() {
  // Already set up? Then this screen is done — go to login (or the app).
  if (await accountExists()) {
    const h = await headers();
    const session = await getSession(h);
    redirect(session ? "/" : "/login");
  }
  return <SetupForm />;
}
