import { BaseStorage, type Storage } from "@evex/linejs/storage";
import { readFile, mkdir, open, rename } from "node:fs/promises";
import { dirname } from "node:path";

export class AuthStorage extends BaseStorage {
  private data: Record<string, Storage["Value"]> = {};
  private queue: Promise<void> = Promise.resolve();
  private pending = 0;
  private failure?: Error;

  constructor(private path: string, private onFailure: (error: Error) => void = () => {}) { super(); }

  async load(): Promise<void> {
    try {
      const data = JSON.parse((await readFile(this.path, "utf8")).replace(/^\uFEFF/, ""));
      if (!data || typeof data !== "object" || Array.isArray(data)) throw new Error("InvalidAuthStorage");
      this.data = data;
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code !== "ENOENT") throw error;
    }
  }

  async get(key: string): Promise<Storage["Value"] | undefined> {
    if (this.failure) throw this.failure;
    return this.data[key];
  }
  set(key: string, value: Storage["Value"]): Promise<void> {
    return this.save(data => { data[key] = value; });
  }
  delete(key: string): Promise<void> { return this.save(data => { delete data[key]; }); }
  clear(): Promise<void> { return this.save(data => { for (const key of Object.keys(data)) delete data[key]; }); }
  async flush(): Promise<void> { await this.queue; if (this.failure) throw this.failure; }
  async migrate(storage: BaseStorage): Promise<void> {
    for (const [key, value] of Object.entries(this.data)) await storage.set(key, value);
  }

  private fail(): Error {
    if (!this.failure) {
      this.failure = new Error("AuthStorageError");
      this.onFailure(this.failure);
    }
    return this.failure;
  }

  private save(change: (data: Record<string, Storage["Value"]>) => void): Promise<void> {
    if (this.failure) return Promise.reject(this.failure);
    if (this.pending >= 64) return Promise.reject(this.fail());
    this.pending++;
    const operation = this.queue.then(async () => {
      if (this.failure) throw this.failure;
      const data = { ...this.data };
      change(data);
      await mkdir(dirname(this.path), { recursive: true, mode: 0o700 });
      const file = await open(`${this.path}.tmp`, "w", 0o600);
      try { await file.writeFile(JSON.stringify(data)); await file.sync(); }
      finally { await file.close(); }
      await rename(`${this.path}.tmp`, this.path);
      this.data = data;
    }).catch(() => { throw this.fail(); }).finally(() => { this.pending--; });
    // 保存失敗後は全体停止へ伝え、以後の認証・sequenceを進めない。
    this.queue = operation;
    void operation.catch(() => {});
    return operation;
  }
}
