const fs = require("node:fs");
const path = require("node:path");
const { spawnSync } = require("node:child_process");
const { pathToFileURL } = require("node:url");

// 配備用fileを作るときだけ、固定したsourceとlockをこの位置へ埋め込む。
const assets = /* PROBE_ASSETS */ null;
const packageDir = "/tmp/kbc-linejs-receiver";
const runId = process.env.PROBE_RUN_ID || "receiver-20261001-a";

function runNode(script, args = []) {
  const result = spawnSync(process.execPath, [path.join(packageDir, script), ...args], {
    cwd: "/app", env: process.env, stdio: "inherit", timeout: 180000,
  });
  if (result.error || result.status !== 0) throw new Error("StorageHelperFailed");
}

(async () => {
  if (!assets) throw new Error("BuildRuntimeFileFirst");
  if (!/^[A-Za-z0-9_-]{1,64}$/.test(runId)) throw new Error("InvalidRunId");
  fs.mkdirSync(packageDir, { recursive: true, mode: 0o700 });
  for (const [name, data] of Object.entries(assets)) {
    if (!/^(package\.json|package-lock\.json|\.npmrc|live-probe\.mjs|legacy-storage\.mjs)$/.test(name)) {
      throw new Error("UnexpectedAsset");
    }
    fs.writeFileSync(path.join(packageDir, name), Buffer.from(data, "base64"), { mode: 0o600 });
  }
  // 旧configが参照する相対pathは、起動時の/appを基準に固定する。
  process.env.LINE_STORAGE_FILE = path.resolve("/app", process.env.LINE_STORAGE_FILE || "./storage/storage.json");
  Object.assign(process.env, {
    LEGACY_APP_DIR: "/app", PROBE_RUN_ID: runId,
    PROBE_OUTPUT_DIR: `/app/logs/receiver-probe-${runId}`,
    PROBE_INTERVAL_MS: "1000", PROBE_MAX_REQUESTS: "400", PROBE_DURATION_SECONDS: "300",
  });
  runNode("legacy-storage.mjs", ["restore", "--old-receiver-stopped"]);
  const install = spawnSync("npm", ["ci", "--ignore-scripts", "--no-audit", "--no-fund"], {
    cwd: packageDir, env: process.env, stdio: "inherit", timeout: 180000,
  });
  if (install.error || install.status !== 0) throw new Error("DependencyInstallFailed");
  process.argv.push("--live", "--old-receiver-stopped");
  await import(pathToFileURL(path.join(packageDir, "live-probe.mjs")));
  // Volumeがないため、更新した認証は終了直後に既存backupへ保存する。
  runNode("legacy-storage.mjs", ["save-auth", "--old-receiver-stopped"]);
  console.log(JSON.stringify({ kind: "bootstrap-finished", runId }));
})().catch((error) => {
  console.error(JSON.stringify({ kind: "bootstrap-error", code: /^[A-Za-z0-9_-]{1,80}$/.test(error.message) ? error.message : "BootstrapFailed" }));
  // 起動失敗から再ログインを繰り返さず、管理者が停止・復旧するまで待機する。
  setInterval(() => {}, 60000);
});
