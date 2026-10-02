const fs = require("node:fs");
const path = require("node:path");
const crypto = require("node:crypto");

// 旧Botの公開検索データだけを、配備用の一つのsnapshotへ変換する。
const source = process.argv[2];
if (!source) throw new Error("Specify the legacy data/search directory");
const read = name => fs.readFileSync(path.join(source, name), "utf8").replace(/^\uFEFF/, "");
const categories = { 0: "N", 1: "S", 2: "C", 4: "E", 6: "T", 7: "V", 11: "R", 12: "M", 13: "NA", 14: "B", 16: "D", 20: "0Z", 21: "1Z", 22: "2Z", 23: "2_Inv", 24: "A", 25: "H", 27: "CA", 30: "DM", 31: "Q", 33: "L", 34: "ND", 36: "SR", 37: "G", 38: "2Z_Inv" };
const pad = n => String(n).padStart(3, "0");
function mapId(raw) {
  if (raw >= 3000 && raw <= 3008) return `${Math.floor((raw - 3000) / 3)}${pad((raw - 3000) % 3)}`;
  if (raw >= 20000 && raw <= 22002) return `${Math.floor((raw - 20000) / 1000)}Z${pad(raw % 1000)}`;
  if (raw === 23000) return "2_Inv000";
  if (raw === 38000) return "2Z_Inv000";
  return `${categories[Math.floor(raw / 1000)] ?? Math.floor(raw / 1000)}${pad(raw % 1000)}`;
}
function stageEntry(id, name, raw, stage) {
  const type = id.slice(0, -3);
  const index = Number(id.slice(-3));
  const special = raw >= 3000 && raw <= 3008 || raw >= 20000 && raw <= 22002;
  const suffix = stage === undefined ? "" : `-${pad(stage)}`;
  return { id: `${special ? raw : id}${suffix}`, names: [name],
    lookupIds: [id + suffix, ...(raw >= 0 ? [String(raw) + suffix] : [])],
    url: `https://jarjarblink.github.io/JDB/map.html?cc=ja&type=${type}&map=${index}${stage === undefined ? "" : `&stage=${stage}`}` };
}
const maps = [];
for (const line of read("Map_Name.csv").split(/\r?\n/)) {
  const comma = line.indexOf(",");
  if (comma < 0) continue;
  const raw = Number(line.slice(0, comma));
  const name = line.slice(comma + 1).trim();
  if (!Number.isSafeInteger(raw) || raw < 0 || !name || ["@", "＠"].includes(name)) continue;
  maps.push(stageEntry(mapId(raw), name, raw));
}
const stages = [];
const files = fs.readdirSync(source).filter(file => /^StageName_?[A-Za-z0-9]+_ja\.csv$/.test(file)).sort();
for (const file of files) {
  const category = file.replace(/^StageName_?/, "").replace(/_ja\.csv$/, "");
  if (category === "3") continue;
  const numeric = /^\d+$/.test(category);
  read(file).split(/\r?\n/).forEach((line, index) => {
    const raw = ["0", "1", "2"].includes(category) ? 3000 + Number(category) * 3 + index
      : numeric ? Number(category) * 1000 + index : -1;
    const id = raw >= 0 ? mapId(raw) : category + pad(index);
    // 空欄や終端記号を飛ばしても、本来の列番号をstage IDに使う。
    line.split(",").forEach((value, stage) => {
      const name = value.trim();
      if (name && !["@", "＠"].includes(name)) stages.push(stageEntry(id, name, raw, stage));
    });
  });
}
const entities = name => JSON.parse(read(name)).sort((a, b) => a.id - b.id)
  .map(row => ({ id: String(row.id), names: row.names, lookupIds: [String(row.id)], url: row.url }));
const entries = { ut: entities("charaname.json"), tut: entities("enemyname.json"), st: [...maps, ...stages] };
const payload = JSON.stringify(entries);
const catalog = { revision: crypto.createHash("sha256").update(payload).digest("hex"),
  source: process.argv[3] ?? "KBC-rakv0-line-bot/data/search (manual snapshot)", entries };
const output = path.resolve(__dirname, "../data/search/catalog.json");
fs.mkdirSync(path.dirname(output), { recursive: true });
fs.writeFileSync(output, "\ufeff" + JSON.stringify(catalog) + "\n");
console.log(JSON.stringify({ output, bytes: fs.statSync(output).size, counts: Object.fromEntries(Object.entries(entries).map(([key, rows]) => [key, rows.length])), revision: catalog.revision }));
