// AuthStorage — the symbols pi-web imports from `pi-coding-agent`'s auth layer.
//
// The catalyst-code core resolves API keys from (1) an explicit `/login` paste
// or `set_key`, (2) this app's OAuth store under `~/.config/catalyst-code/oauth/`.
// A fresh install with no configured provider does not scan env vars; a
// provider explicitly configured with `api_key_env` reads the env var at
// request time. Third-party CLI credential files are not used for auth.
// This class is the bridge: keys set here are forwarded to a spawned core via
// `set_key` (handled by `AgentSession`).

import type { AuthCredential } from "./auth-types.js";

export type {
  ApiKeyCredential,
  OAuthCredential,
  AuthCredential,
  AuthStatus,
  AuthStorageData,
  AuthStorageBackend,
} from "./auth-types.js";
import {
  InMemoryAuthStorageBackend,
  FileAuthStorageBackend,
  type AuthStatus,
  type AuthStorageData,
} from "./auth-types.js";

/** A callback registered by an AgentSession to receive runtime key updates. */
export type RuntimeKeySink = (provider: string | undefined, apiKey: string) => void;

export class AuthStorage {
  readonly backend: AuthStorageBackendLike;
  private runtimeKeys = new Map<string, string>();
  private fallbackResolver?: (provider: string) => string | undefined;
  private sinks = new Set<RuntimeKeySink>();

  private constructor(backend: AuthStorageBackendLike) {
    this.backend = backend;
  }

  /** Default factory — backed by an in-memory store (the core owns persistence). */
  static create(_authPath?: string): AuthStorage {
    return new AuthStorage(new InMemoryAuthStorageBackend());
  }

  static fromStorage(backend: AuthStorageBackendLike): AuthStorage {
    return new AuthStorage(backend);
  }

  static inMemory(data?: AuthStorageData): AuthStorage {
    return new AuthStorage(new InMemoryAuthStorageBackend(data));
  }

  /** Register a sink that receives runtime key updates (called by AgentSession). */
  _addSink(sink: RuntimeKeySink): () => void {
    this.sinks.add(sink);
    return () => this.sinks.delete(sink);
  }

  setRuntimeApiKey(provider: string, apiKey: string): void {
    this.runtimeKeys.set(provider, apiKey);
    for (const sink of this.sinks) sink(provider, apiKey);
  }

  removeRuntimeApiKey(provider: string): void {
    this.runtimeKeys.delete(provider);
  }

  setFallbackResolver(resolver: (provider: string) => string | undefined): void {
    this.fallbackResolver = resolver;
  }

  reload(): void {
    /* no-op: the core owns persistence; runtime keys are in-memory. */
  }

  get(provider: string): AuthCredential | undefined {
    const key = this.runtimeKeys.get(provider);
    if (key) return { type: "api_key", key };
    return (this.backend.read() as AuthStorageData)[provider];
  }

  set(provider: string, credential: AuthCredential): void {
    if (credential.type === "api_key") this.setRuntimeApiKey(provider, credential.key);
    else this.backend.write({ ...(this.backend.read() as AuthStorageData), [provider]: credential });
  }

  remove(provider: string): void {
    this.runtimeKeys.delete(provider);
    const data = { ...(this.backend.read() as AuthStorageData) };
    delete data[provider];
    this.backend.write(data);
  }

  list(): string[] {
    return Array.from(new Set([...this.runtimeKeys.keys(), ...Object.keys(this.backend.read() as AuthStorageData)]));
  }

  has(provider: string): boolean {
    return this.runtimeKeys.has(provider) || provider in (this.backend.read() as AuthStorageData);
  }

  hasAuth(provider: string): boolean {
    return this.has(provider);
  }

  getAuthStatus(provider: string): AuthStatus {
    const configured = this.has(provider);
    return {
      configured,
      source: configured ? (this.runtimeKeys.has(provider) ? "runtime" : "stored") : undefined,
    };
  }

  getAll(): AuthStorageData {
    const out: AuthStorageData = { ...(this.backend.read() as AuthStorageData) };
    for (const [p, k] of this.runtimeKeys) out[p] = { type: "api_key", key: k };
    return out;
  }

  drainErrors(): Error[] {
    return [];
  }

  async getApiKey(providerId: string, _options?: { includeFallback?: boolean }): Promise<string | undefined> {
    return this.runtimeKeys.get(providerId) ?? this.fallbackResolver?.(providerId);
  }

  getOAuthProviders(): any[] {
    return [];
  }
  async login(_providerId: string, _callbacks: any): Promise<void> {
    throw new Error("OAuth login is not supported by the catalyst-code SDK");
  }
  logout(_provider: string): void {}
}

export type AuthStorageBackendLike = {
  read(): AuthStorageData;
  write(data: AuthStorageData): void;
};

export { FileAuthStorageBackend, InMemoryAuthStorageBackend };
