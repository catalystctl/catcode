// Auth credential types — mirrors `pi-coding-agent`'s `core/auth-storage.ts`.

export interface ApiKeyCredential {
  type: "api_key";
  key: string;
}
export interface OAuthCredential {
  type: "oauth";
}
export type AuthCredential = ApiKeyCredential | OAuthCredential;
export type AuthStorageData = Record<string, AuthCredential>;

export interface AuthStatus {
  configured: boolean;
  source?: "stored" | "runtime" | "environment" | "fallback" | "models_json_key" | "models_json_command";
  label?: string;
}

export interface AuthStorageBackend {
  withLock<T>(fn: () => T): T;
  withLockAsync<T>(fn: () => Promise<T>): Promise<T>;
}

export class FileAuthStorageBackend implements AuthStorageBackend {
  constructor(public authPath?: string) {}
  withLock<T>(fn: () => T): T {
    return fn();
  }
  withLockAsync<T>(fn: () => Promise<T>): Promise<T> {
    return fn();
  }
}

export class InMemoryAuthStorageBackend implements AuthStorageBackend {
  private data: AuthStorageData;
  constructor(data: AuthStorageData = {}) {
    this.data = data;
  }
  withLock<T>(fn: () => T): T {
    return fn();
  }
  withLockAsync<T>(fn: () => Promise<T>): Promise<T> {
    return fn();
  }
  read(): AuthStorageData {
    return this.data;
  }
  write(data: AuthStorageData): void {
    this.data = data;
  }
}
