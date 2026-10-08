import { readFile } from "node:fs/promises";
import type { RuntimeStatus } from "../protocol/native.js";
import type { ApiScheduler } from "./api.js";

type FileReader = (path: string) => Promise<string>;
type ResourceStatus = Pick<RuntimeStatus, "sampledAtMs" | "cpuCorePercent" | "cpuLimitCores" | "cpuSampleSeconds" |
  "cpuContainer" | "memoryUsedBytes" | "memoryLimitBytes" | "memoryContainer" | "processRssBytes">;

function positive(value: string | undefined): number | null {
  const parsed = Number(value);
  return Number.isFinite(parsed) && parsed > 0 && parsed < Number.MAX_SAFE_INTEGER ? parsed : null;
}

// cgroupの上限はホストのCPU数・RAMとは異なる。未設定は推測せずnullにする。
export async function readContainerResources(reader: FileReader = path => readFile(path, "utf8")) {
  const paths = ["cpu.stat", "cpu.max", "memory.current", "memory.max", "cpuacct/cpuacct.usage",
    "cpu/cpu.cfs_quota_us", "cpu/cpu.cfs_period_us", "memory/memory.usage_in_bytes", "memory/memory.limit_in_bytes"];
  const values = await Promise.all(paths.map(path => reader(`/sys/fs/cgroup/${path}`).catch(() => "")));
  const [stat, max, used, limit, oldCpu, quota, period, oldUsed, oldLimit] = values;
  const usage = stat.match(/^usage_usec\s+(\d+)$/m)?.[1];
  const [cpuQuota, cpuPeriod] = max.trim().split(/\s+/);
  const current = positive(cpuQuota), interval = positive(cpuPeriod);
  const oldQuota = positive(quota), oldPeriod = positive(period);
  return {
    cpuMicros: usage === undefined ? positive(oldCpu) === null ? null : Number(oldCpu) / 1000 : Number(usage),
    cpuLimitCores: max.trim() ? current && interval ? current / interval : null : oldQuota && oldPeriod ? oldQuota / oldPeriod : null,
    memoryUsedBytes: positive(used) ?? positive(oldUsed),
    memoryLimitBytes: limit.trim() ? positive(limit) : positive(oldLimit),
  };
}

async function buildInfo(): Promise<Pick<RuntimeStatus, "branch" | "commit" | "dirty">> {
  let stored: { branch?: string; commit?: string; dirty?: boolean } = {};
  try { stored = JSON.parse(await readFile(new URL("../../native/build-info.json", import.meta.url), "utf8")); }
  catch { /* Dockerではビルド時に固定した環境変数を使用する。 */ }
  const branch = process.env.BOT_BUILD_BRANCH || stored.branch;
  const commit = process.env.BOT_BUILD_COMMIT || process.env.NF_DEPLOYMENT_SHA || stored.commit;
  return { branch: branch && /^[A-Za-z0-9_./-]{1,128}$/.test(branch) ? branch : null,
    commit: commit && /^[a-f0-9]{7,40}$/i.test(commit) ? commit : null,
    dirty: !process.env.BOT_BUILD_COMMIT && !process.env.NF_DEPLOYMENT_SHA && stored.dirty === true };
}

export class RuntimeMonitor {
  private resources: ResourceStatus = { sampledAtMs: 0, cpuCorePercent: null, cpuLimitCores: null, cpuSampleSeconds: 0,
    cpuContainer: false, memoryUsedBytes: 0, memoryLimitBytes: null, memoryContainer: false, processRssBytes: 0 };
  private revision: Pick<RuntimeStatus, "branch" | "commit" | "dirty"> = { branch: null, commit: null, dirty: false };
  private previous?: { at: number; cpu: number; container: boolean };
  private refreshing = false;

  constructor(private gate: ApiScheduler, private queryWorkers: number, private deliveryWorkers: number,
    private receiver: () => { state: string; chats: number }) {}

  async initialize(): Promise<void> {
    this.revision = await buildInfo();
    await this.refresh();
  }

  async refresh(): Promise<void> {
    if (this.refreshing) return;
    this.refreshing = true;
    try {
      const container = await readContainerResources();
      const processCpu = process.cpuUsage();
      const at = performance.now();
      const cpu = container.cpuMicros ?? processCpu.user + processCpu.system;
      const cpuContainer = container.cpuMicros !== null;
      const seconds = this.previous && this.previous.container === cpuContainer ? (at - this.previous.at) / 1000 : 0;
      const percent = seconds > 0 && this.previous && cpu >= this.previous.cpu ? (cpu - this.previous.cpu) / (seconds * 10000) : null;
      const rss = process.memoryUsage().rss;
      this.resources = { sampledAtMs: Date.now(), cpuCorePercent: percent, cpuLimitCores: container.cpuLimitCores,
        cpuSampleSeconds: seconds, cpuContainer, memoryUsedBytes: container.memoryUsedBytes ?? rss,
        memoryLimitBytes: container.memoryUsedBytes === null ? null : container.memoryLimitBytes,
        memoryContainer: container.memoryUsedBytes !== null, processRssBytes: rss };
      this.previous = { at, cpu, container: cpuContainer };
    } finally { this.refreshing = false; }
  }

  // 受付・healthではローカルの最新値だけを読む。LINE照会やファイル読込をしない。
  snapshot(): RuntimeStatus {
    const receiver = this.receiver();
    return { ...this.resources, ...this.revision, ...this.gate.snapshot(), queryWorkers: this.queryWorkers,
      deliveryWorkers: this.deliveryWorkers, receiverState: receiver.state, joinedChats: receiver.chats };
  }
}
