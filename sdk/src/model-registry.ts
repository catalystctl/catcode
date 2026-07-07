// ModelRegistry — mirrors `pi-coding-agent`'s `core/model-registry.ts`.
//
// Unlike PI (built-in model table + models.json), catalyst-code discovers
// models dynamically from the active provider inside the core binary, which
// reports them via the `ready`/`models` events. This registry is a passive
// container: a `CoreProcess` populates it when models arrive, and
// `AgentSession`/consumers read from it. A process-wide shared singleton backs
// `getModel()`/`getModels()` in `ai.ts`.

import { AuthStorage } from "./auth-storage.js";
import type { AuthStatus } from "./auth-types.js";
import type { Api, Model } from "./types.js";

export type ResolvedRequestAuth =
  | { ok: true; apiKey?: string; headers?: Record<string, string> }
  | { ok: false; error: string };

export class ModelRegistry {
  readonly authStorage: AuthStorage;
  private models: Model<any>[] = [];
  private error: string | undefined;
  private modelsJsonPath?: string;

  private constructor(authStorage: AuthStorage, modelsJsonPath?: string) {
    this.authStorage = authStorage;
    this.modelsJsonPath = modelsJsonPath;
  }

  static create(authStorage: AuthStorage, modelsJsonPath?: string): ModelRegistry {
    return new ModelRegistry(authStorage, modelsJsonPath);
  }

  static inMemory(authStorage: AuthStorage): ModelRegistry {
    return new ModelRegistry(authStorage);
  }

  /** Re-read configuration. No-op for the wrapper (models arrive from the core). */
  refresh(): void {
    /* no-op */
  }

  getError(): string | undefined {
    return this.error;
  }

  /** Internal: populate models discovered by a core process. */
  _setModels(models: Model<any>[]): void {
    this.models = models;
    this.error = undefined;
  }

  /** Internal: merge a provider's discovered models (dedup by provider+id). */
  _mergeModels(models: Model<any>[]): void {
    const seen = new Set(this.models.map((m) => `${m.provider}/${m.id}`));
    for (const m of models) {
      const key = `${m.provider}/${m.id}`;
      if (!seen.has(key)) {
        this.models.push(m);
        seen.add(key);
      }
    }
  }

  _setError(error: string | undefined): void {
    this.error = error;
  }

  /** All known models (built-in/registered). */
  getAll(): Model<any>[] {
    return [...this.models];
  }

  /** Models with auth configured. The wrapper treats all discovered models as
   *  available (the core validates auth per request). */
  getAvailable(): Model<any>[] {
    return [...this.models];
  }

  find(provider: string, modelId: string): Model<any> | undefined {
    return this.models.find((m) => m.provider === provider && m.id === modelId)
      ?? this.models.find((m) => m.id === modelId);
  }

  hasConfiguredAuth(model: Model<any>): boolean {
    return this.authStorage.hasAuth(model.provider);
  }

  async getApiKeyAndHeaders(model: Model<any>): Promise<ResolvedRequestAuth> {
    const key = await this.authStorage.getApiKey(model.provider);
    if (key) return { ok: true, apiKey: key };
    // The core may resolve the key from env/config; surface a soft ok so callers
    // don't block on a missing runtime key.
    return { ok: true };
  }

  getProviderAuthStatus(provider: string): AuthStatus {
    return this.authStorage.getAuthStatus(provider);
  }

  getProviderDisplayName(provider: string): string {
    return provider;
  }

  async getApiKeyForProvider(provider: string): Promise<string | undefined> {
    return this.authStorage.getApiKey(provider);
  }

  isUsingOAuth(_model: Model<any>): boolean {
    return false;
  }

  registerProvider(_providerName: string, _config: any): void {
    /* custom providers are configured via UMANS_PROVIDERS env / core config */
  }

  unregisterProvider(providerName: string): void {
    this.models = this.models.filter((m) => m.provider !== providerName);
  }
}

// ── Shared singleton (process-wide) ────────────────────────────────────────

let sharedAuthStorage: AuthStorage | null = null;
let sharedModelRegistry: ModelRegistry | null = null;

export function getSharedAuth(): AuthStorage {
  if (!sharedAuthStorage) sharedAuthStorage = AuthStorage.create();
  return sharedAuthStorage;
}

export function getSharedModelRegistry(): ModelRegistry {
  if (!sharedModelRegistry) {
    sharedModelRegistry = ModelRegistry.create(getSharedAuth());
    sharedModelRegistry.refresh();
  }
  return sharedModelRegistry;
}

/** Test helper: reset the shared singletons between tests. */
export function _resetSharedRegistries(): void {
  sharedAuthStorage = null;
  sharedModelRegistry = null;
}
