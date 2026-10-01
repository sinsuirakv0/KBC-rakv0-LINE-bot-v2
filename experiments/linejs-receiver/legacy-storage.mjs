import { readFile } from "node:fs/promises";
import { resolve } from "node:path";
import { pathToFileURL } from "node:url";

// 旧imageのStorageだけを使い、旧Bot本体やLINE受信器は起動しない。
async function run() {
  if (!process.argv.includes("--old-receiver-stopped")) throw new Error("OldReceiverMustBeStopped");
  const mode = process.argv[2];
  if (!["restore", "save-auth"].includes(mode)) throw new Error("InvalidMode");
  const legacyDir = resolve(process.env.LEGACY_APP_DIR || "/app");
  const { initializeLineStorage } = await import(pathToFileURL(resolve(legacyDir, "dist/storage/lineStorage.js")));
  const { appConfig } = await import(pathToFileURL(resolve(legacyDir, "dist/config.js")));
  const storage = await initializeLineStorage();
  const backupConfigured = Boolean(appConfig.pushSubscriptionsGithubRepo && appConfig.pushSubscriptionsGithubToken && appConfig.lineStorageBackupKey);
  if (mode === "restore") {
    const token = await storage.get(".auth");
    console.log(JSON.stringify({ kind: "legacy-storage", restored: true,
      hasStoredToken: typeof token === "string" && Boolean(token),
      hasCheckpoint: Boolean(await storage.get("kbc.squareSyncToken")),
      hasOwner: Boolean(await storage.get("kbc.squareSyncOwnerMid")),
      backupConfigured }));
    return;
  }
  if (!process.env.PROBE_OUTPUT_DIR) throw new Error("ProbeOutputDirRequired");
  const outputDir = resolve(process.env.PROBE_OUTPUT_DIR);
  const summary = JSON.parse(await readFile(resolve(outputDir, "summary.json"), "utf8"));
  if (summary.status !== "finished") throw new Error("ProbeMustBeFinished");
  if (!summary.authUpdated) {
    console.log(JSON.stringify({ kind: "save-auth", changed: false }));
    return;
  }
  const probe = JSON.parse(await readFile(resolve(outputDir, "auth.json"), "utf8"));
  if (!probe["kbc.squareSyncOwnerMid"] || probe["kbc.squareSyncOwnerMid"] !== await storage.get("kbc.squareSyncOwnerMid")) {
    throw new Error("AuthOwnerMismatch");
  }
  const keys = [".auth", "refreshToken", "expire"];
  for (const key of keys) {
    const value = probe[key];
    if (value !== undefined && !["string", "number"].includes(typeof value)) throw new Error("InvalidAuthValue");
  }
  if (typeof probe[".auth"] !== "string" || !probe[".auth"]) throw new Error("NoProbeAuthToken");
  // 更新された認証だけを引き継ぎ、旧Botのcheckpoint・機能データは維持する。
  for (const key of keys) {
    if (probe[key] !== undefined) await storage.set(key, probe[key]);
  }
  await storage.flushBackup();
  console.log(JSON.stringify({ kind: "save-auth", changed: true, persistedLocally: true, backupFlushed: backupConfigured }));
}

await run().catch((error) => {
  const code = /^[A-Za-z0-9_-]{1,80}$/.test(error.message) ? error.message : "LegacyStorageFailed";
  console.error(JSON.stringify({ kind: "legacy-storage-error", code }));
  process.exitCode = 1;
});
