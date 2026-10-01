const { spawnSync } = require("node:child_process");
const { resolve } = require("node:path");
const { mkdirSync, writeFileSync, copyFileSync, existsSync } = require("node:fs");
const root = resolve(__dirname, "..");
const compilerDirectory = process.env.LLVM_MINGW_BIN || resolve(root, ".tools/llvm-mingw-20260922-ucrt-x86_64/bin");
if (process.platform === "win32" && existsSync(compilerDirectory)) {
  process.env.PATH = `${compilerDirectory};${process.env.PATH}`;
  process.env.CARGO_TARGET_X86_64_PC_WINDOWS_GNULLVM_LINKER = "x86_64-w64-mingw32-clang";
}
function run(program, args) {
  const result = spawnSync(program, args, { cwd: root, env: process.env, stdio: "inherit" });
  if (result.status !== 0) process.exit(result.status ?? 1);
}
if (process.argv[2] === "generate") {
  run("cargo", ["run", "--locked", "--quiet", "-p", "kbc-protocol", "--bin", "generate_types"]);
} else if (process.argv[2] === "build") {
  const rustc = (...args) => spawnSync("rustc", args, { encoding: "utf8" }).stdout.trim();
  const host = rustc("-vV").match(/^host: (.+)$/m)?.[1];
  // Discord v2と同じWindows GNU LLVM環境に対応する。
  if (host?.endsWith("-gnullvm")) {
    const directory = resolve(root, "target/libnode");
    mkdirSync(directory, { recursive: true });
    writeFileSync(resolve(directory, "libnode.dll"), "");
    writeFileSync(resolve(directory, "libnode.dll.a"), "!<arch>\n", "ascii");
    process.env.LIBNODE_PATH = directory;
  }
  run("cargo", ["build", "--locked", "--release", "-p", "kbc-node"]);
  const library = { win32: "kbc_node.dll", linux: "libkbc_node.so", darwin: "libkbc_node.dylib" }[process.platform];
  if (!library) throw new Error("Unsupported platform");
  mkdirSync(resolve(root, "native"), { recursive: true });
  copyFileSync(resolve(root, "target/release", library), resolve(root, "native/kbc_node.node"));
  if (host?.endsWith("-gnullvm")) {
    copyFileSync(resolve(rustc("--print", "sysroot"), "lib/rustlib", host, "bin/libunwind.dll"), resolve(root, "native/libunwind.dll"));
  }
} else {
  throw new Error("Use generate or build");
}
