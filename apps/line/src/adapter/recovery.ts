import type { BaseClient } from "@evex/linejs/base";
import { setTimeout as delay } from "node:timers/promises";
import { ApiScheduler, errorCode } from "./api.js";

// 復旧の所有者は受信loopだけ。RPCからは停止を通知し、並列の認証更新を起動しない。
export class LineRecovery {
  state: "ready" | "api-recovering" | "api-suspended" = "ready";
  version = 0;
  onPause?: () => void;
  private attempts = 0;
  private nextAttemptAtMs = 0;
  private code = "";
  private refreshing?: Promise<void>;
  private checks = 0;
  private prepared = false;

  constructor(private client: BaseClient, private gate: ApiScheduler, private signal: AbortSignal,
    private flush: () => Promise<void>, private retryMs = 2000, private monitorMs = 60_000) {
    gate.onAuthenticationFailure = error => this.notice(error);
    const refresh = client.auth.tryRefreshToken.bind(client.auth);
    // SDK内部の自動更新も同じ1件へ合流し、refreshTokenの同時rotationを避ける。
    client.auth.tryRefreshToken = () => {
      if (!this.refreshing) this.refreshing = refresh().then(() => flush()).finally(() => { this.refreshing = undefined; });
      return this.refreshing;
    };
  }

  get blocked(): boolean { return this.state !== "ready"; }
  snapshot() { return { state: this.state, attempts: this.attempts, maxAttempts: 3,
    checks: this.checks, nextAttemptAtMs: this.nextAttemptAtMs, code: this.code }; }

  notice(error: unknown): void {
    if (this.signal.aborted || errorCode(error) === "LineApiUnavailable" || this.blocked) return;
    this.state = "api-recovering";
    this.version++;
    this.attempts = 0;
    this.prepared = false;
    this.code = errorCode(error);
    this.nextAttemptAtMs = Date.now();
    this.gate.pause();
    this.onPause?.();
    this.log();
  }

  failed(error: unknown): void {
    if (this.signal.aborted) return;
    this.notice(error);
    this.prepared = false;
    if (errorCode(error) !== "LineApiUnavailable") this.code = errorCode(error);
    if (this.attempts >= 3 || this.state === "api-suspended") {
      this.state = "api-suspended";
      this.nextAttemptAtMs = Date.now() + this.monitorMs;
    } else this.nextAttemptAtMs = Date.now() + this.retryMs;
    this.log();
  }

  async beforeConnect(): Promise<void> {
    if (!this.blocked || this.prepared) return;
    await delay(Math.max(0, this.nextAttemptAtMs - Date.now()), undefined, { signal: this.signal });
    this.signal.throwIfAborted();
    if (this.state === "api-suspended") this.checks++;
    else this.attempts++;
    // 保存済みrefresh情報がない場合は、既存accessTokenでの購読だけを確認する。
    await this.gate.withProbe(async () => {
      if (typeof await this.client.storage.get("refreshToken") === "string") {
        try { await this.client.auth.tryRefreshToken(); }
        catch (error) {
          if (this.signal.aborted || !this.client.authToken) throw error;
          // refresh情報だけが無効な場合も、現在tokenによるSquare購読で解除を確認する。
          console.log(JSON.stringify({ kind: "line-refresh-failure", code: errorCode(error) }));
        }
      }
      await this.flush();
    });
    this.prepared = true;
  }

  healthy(): void {
    if (!this.blocked) return;
    this.state = "ready";
    this.prepared = false;
    this.attempts = 0;
    this.nextAttemptAtMs = 0;
    this.code = "";
    this.gate.resume();
    this.log();
  }

  private log(): void { console.log(JSON.stringify({ kind: "line-recovery", ...this.snapshot() })); }
}
