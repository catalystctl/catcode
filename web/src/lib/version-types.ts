/** Shared version / update-status types (safe for client + server imports). */

export type VersionUpdateStatus =
  | "up_to_date"
  | "out_of_date"
  | "uncommitted"
  | "ahead"
  | "unknown";

export interface EmbeddedVersion {
  commit: string;
  commitFull?: string;
  dirty?: boolean;
  builtAt?: string;
  source?: "release" | "source" | "dev" | string;
}

export interface VersionInfo {
  commit: string;
  commitFull: string;
  dirty: boolean;
  builtAt: string | null;
  source: string;
  latest: string | null;
  latestFull: string | null;
  status: VersionUpdateStatus;
  statusLabel: string;
  repo: string;
  commitUrl: string | null;
  latestUrl: string | null;
}
