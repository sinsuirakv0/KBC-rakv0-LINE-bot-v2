import { BaseStorage, type Storage } from "@evex/linejs/storage";
import { readFile, mkdir, open, rename } from "node:fs/promises";
import { dirname } from "node:path";

export class AuthStorage extends BaseStorage {
  private data: Record<string, Storage["Value"]> = {};
  private queue: Promise<void> = Promise.resolve();

  constructor(private path: string) { super(); }

  async load(): Promise<void> {
    try {
      const data = JSON.parse((await readFile(this.path, "utf8")).replace(/^\uFEFF/, ""));
      if (!data || typeof data !== "object" || Array.isArray(data)) throw new Error("InvalidAuthStorage");
      this.data = data;
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code !== "ENOENT") throw error;
    }
  }

  async get(key: string): Promise<Storage["Value"] | undefined> { return this.data[key]; }
  set(key: string, value: Storage["Value"]): Promise<void> {
    return this.save(() => { this.data[key] = value; });
  }
  delete(key: string): Promise<void> { return this.save(() => { delete this.data[key]; }); }
  clear(): Promise<void> { return this.save(() => { this.data = {}; }); }
  async migrate(storage: BaseStorage): Promise<void> {
    for (const [key, value] of Object.entries(this.data)) await storage.set(key, value);
  }

  private save(change: () => void): Promise<void> {
    const operation = this.queue.then(async () => {
      change();
      await mkdir(dirname(this.path), { recursive: true, mode: 0o700 });
      const file = await open(`${this.path}.tmp`, "w", 0o600);
      try { await file.writeFile(JSON.stringify(this.data)); await file.sync(); }
      finally { await file.close(); }
      await rename(`${this.path}.tmp`, this.path);
    });
    this.queue = operation;
    return operation;
  }
}
