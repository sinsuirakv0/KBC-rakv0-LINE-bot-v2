import { spawn } from "node:child_process";
import { fileURLToPath } from "node:url";
import { setTimeout as delay } from "node:timers/promises";
import { errorCode } from "./api.js";

const script = fileURLToPath(new URL("../../scripts/search-snapshot.cjs", import.meta.url));

// 公開資料の更新はPUSH受付から分離する。各回の変換後は子プロセスごとメモリを解放する。
export class SearchDataUpdater {
  readonly metrics = { refreshes: 0, failures: 0, lastSuccessAt: 0 };

  constructor(private output: string) {}

  async refresh(signal: AbortSignal): Promise<void> {
    if (signal.aborted) return;
    const child = spawn(process.execPath, [script, this.output, "--live"], {
      stdio: ["ignore", "ignore", "pipe"], signal, timeout: 90000, windowsHide: true,
    });
    let detail = "";
    child.stderr.on("data", (chunk: Buffer) => { detail = (detail + chunk.toString()).slice(-2048); });
    try {
      await new Promise<void>((resolve, reject) => {
        child.once("error", reject);
        child.once("close", code => code === 0 ? resolve() : reject(new Error(detail || `SearchRefreshExit${code}`)));
      });
      this.metrics.refreshes++;
      this.metrics.lastSuccessAt = Date.now();
    } catch (error) {
      if (signal.aborted) return;
      this.metrics.failures++;
      console.error(JSON.stringify({ kind: "searchDataRefreshFailure", code: errorCode(error) }));
    }
  }

  async run(signal: AbortSignal): Promise<void> {
    while (!signal.aborted) {
      const startedAt = Date.now();
      await this.refresh(signal);
      try { await delay(Math.max(1000, 90000 - (Date.now() - startedAt)), undefined, { signal }); }
      catch (error) { if (!signal.aborted) throw error; }
    }
  }
}
