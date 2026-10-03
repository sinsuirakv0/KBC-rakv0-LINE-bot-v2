import json
import subprocess
from pathlib import Path

# cgroupのpeakは子FFmpegとfile cacheも含み、短い瞬間の増加も保持する。
results = []
root = Path.cwd()
for kind, baseline in (("mp4", 110), ("gif", 110), ("png", 110), ("mp4", 420)):
    name = f"kbc-motion-probe-{kind}-{baseline}"
    subprocess.run(["docker", "create", "--name", name, "--cpus", "0.2", "--memory", "512m", "--memory-swap", "512m",
                    "--env", f"MOTION_FORMAT={kind}", "--env", f"MOTION_BASELINE_MIB={baseline}", "--volume", f"{root}/experiments/motion-memory:/app/experiments/motion-memory:ro",
                    "--entrypoint", "node", "kbc-motion-probe", "/app/experiments/motion-memory/run.mjs"], check=True, capture_output=True)
    run = subprocess.run(["docker", "start", "--attach", name], capture_output=True, text=True, timeout=720)
    state = json.loads(subprocess.check_output(["docker", "inspect", "--format", "{{json .State}}", name]))
    parsed = [json.loads(line) for line in run.stdout.splitlines() if line.startswith("{")]
    result = {"format": kind, "baselineMiB": baseline, "expectedExitCode": 1 if baseline == 420 else 0,
              "exitCode": state["ExitCode"], "oomKilled": state["OOMKilled"],
              "measurement": parsed[-1] if parsed else None, "stderr": run.stderr[-8192:]}
    results.append(result)
    print(json.dumps(result, ensure_ascii=False), flush=True)
    Path("motion-memory-results.json").write_text(json.dumps(results, ensure_ascii=False, indent=2), encoding="utf-8")
    subprocess.run(["docker", "rm", name], check=True, capture_output=True)
if any(result["exitCode"] != result["expectedExitCode"] or result["oomKilled"]
       or result["baselineMiB"] == 420 and "MediaMemoryBudgetExceeded" not in result["stderr"] for result in results):
    raise SystemExit(1)
