import { auth, ensureAuthReady } from "@/lib/auth";
import { toNextJsHandler } from "better-auth/next-js";

const handlers = toNextJsHandler(auth);

export async function GET(req: Request) {
  await ensureAuthReady();
  return handlers.GET(req);
}

export async function POST(req: Request) {
  await ensureAuthReady();
  return handlers.POST(req);
}
