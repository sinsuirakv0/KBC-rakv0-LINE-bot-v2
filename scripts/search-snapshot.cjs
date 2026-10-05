const fs = require('node:fs');
const path = require('node:path');
const crypto = require('node:crypto');

// Discord版と同じ資料・検証・ID経路。通常は固定版、--liveはmainを条件付き取得する。
const repos = ['sinsuirakv0/KBC-rakv0-assets', 'sinsuirakv0/KBC-rakv0-event', 'Sugar2550/omoroirie'];
const live = process.argv.includes('--live');
const output = path.resolve(process.argv.slice(2).find(arg => arg !== '--live') ?? path.join(__dirname, '../data/search/catalog.json'));
const sourceDirectory = `${output}.sources`;
const usedSources = new Set();
const check = (ok, message) => { if (!ok) throw new Error(message); };
const pad = n => String(n).padStart(3, '0');
const integer = value => { check(/^\d+$/.test(value) && Number.isSafeInteger(Number(value)), 'Invalid integer'); return Number(value); };
const lines = text => text.replace(/^\uFEFF/, '').replace(/\r\n/g, '\n').replace(/\n+$/, '').split('\n');
const name = value => { const text = value.trim(); check(text.length <= 200, 'Name too long'); return text && !['@', '＠'].includes(text) ? text : null; };
async function read(url, optional = false) {
  const cachePath = path.join(sourceDirectory, crypto.createHash('sha256').update(url).digest('hex') + '.json');
  usedSources.add(path.basename(cachePath));
  check(usedSources.size <= 128, 'Too many sources');
  let cached;
  if (live) {
    try { if (fs.statSync(cachePath).size <= 12 * 1024 * 1024) cached = JSON.parse(fs.readFileSync(cachePath, 'utf8')); } catch { /* 未取得・破損時は全量取得する。 */ }
  }
  const headers = { 'User-Agent': 'KBC-LINE-v2-snapshot', 'Cache-Control': 'no-cache' };
  if (typeof cached?.etag === 'string') headers['If-None-Match'] = cached.etag;
  const res = await fetch(url, { signal: AbortSignal.timeout(20000), headers });
  if (res.status === 304 && typeof cached?.text === 'string') return cached.text;
  if (optional && res.status === 404) { if (live) fs.rmSync(cachePath, { force: true }); return null; }
  check(res.ok, `HTTP ${res.status}: ${url}`); let size = 0; const chunks = [];
  for await (const chunk of res.body) { size += chunk.length; check(size <= 10 * 1024 * 1024, 'Source too large'); chunks.push(chunk); }
  const text = new TextDecoder('utf-8', { fatal: true }).decode(Buffer.concat(chunks)).replace(/^\uFEFF/, '');
  if (live) {
    fs.mkdirSync(sourceDirectory, { recursive: true });
    const files = fs.readdirSync(sourceDirectory).filter(file => /^[a-f0-9]{64}\.json$/.test(file));
    if (!fs.existsSync(cachePath) && files.length >= 128) {
      const unused = files.find(file => !usedSources.has(file));
      if (unused) fs.rmSync(path.join(sourceDirectory, unused));
    }
    fs.writeFileSync(cachePath + '.tmp', JSON.stringify({ etag: res.headers.get('etag'), text }));
    fs.renameSync(cachePath + '.tmp', cachePath);
  }
  return text;
}
async function map(items, operation) {
  const result = new Array(items.length); let cursor = 0;
  await Promise.all(Array.from({ length: Math.min(4, items.length) }, async () => {
    while (cursor < items.length) { const i = cursor++; result[i] = await operation(items[i]); }
  })); return result;
}
function idNames(text, negative = false) {
  const result = new Map(); const rows = lines(text); check(rows.length <= 100000, 'Too many rows');
  for (const row of rows) {
    const comma = row.indexOf(','); check(comma >= 0, 'Missing name column'); const raw = row.slice(0, comma).trim();
    if (negative && raw.startsWith('-')) continue;
    const id = integer(raw), value = name(row.slice(comma + 1));
    if (value) { check(!result.has(id), 'Duplicate ID'); result.set(id, value); }
  } check(result.size > 0, 'Empty names'); return result;
}
function stageRows(text) {
  const rows = lines(text); check(rows.length <= 100000, 'Too many rows');
  const result = rows.map(row => { const cells = row.split(','); check(cells.length <= 1000, 'Too many columns'); return cells.map(name); });
  check(result.some(row => row.some(Boolean)), 'Empty stages'); return result;
}
async function main() {
  const startedAt = Date.now();
  const commits = live ? repos.map(() => 'main') : await map(repos, async repo => { const sha = JSON.parse(await read(`https://api.github.com/repos/${repo}/commits/main`)).sha; check(/^[0-9a-f]{40}$/.test(sha), 'Invalid commit'); return sha; });
  const [asset, event, legacy] = repos.map((repo, i) => `https://raw.githubusercontent.com/${repo}/${commits[i]}`);
  const site = `${asset}/jp/sitedata`;
  const [chars, buy, enemy, aliases, types, mapNames, saleNames] = await map([
    `${site}/character-index.json`, `${site}/Data/unitbuy.csv`, `${site}/res/Enemyname.tsv`, `${legacy}/data/enemyname.json`,
    `${event}/data/stage_type.csv`, `${site}/res/Map_Name.csv`, `${event}/data/sale_name.csv`,
  ], url => read(url));
  const shared = lines(buy).filter(row => row.trim()).map(row => {
    const cells = row.split(','); check(cells.length >= 63, 'Invalid UnitBuy');
    return cells.slice(61, 63).map(value => { check(/^-?\d+$/.test(value) && Number(value) >= -1, 'Invalid shared form'); return value === '-1' ? null : pad(integer(value)); });
  });
  const units = JSON.parse(chars).units; check(units.length > 0 && units.length <= shared.length, 'Invalid units');
  const ut = units.map((unit, i) => {
    check(unit.id === pad(i) && unit.forms.length >= 1 && unit.forms.length <= 4 && unit.forms.every(form => typeof form.name === 'string' && form.name.trim()), 'Invalid unit forms');
    check(Array.isArray(unit.aliases) && unit.aliases.every(value => typeof value === 'string' && value.trim()) && new Set(unit.aliases).size === unit.aliases.length, 'Invalid aliases');
    return { id: unit.id, displayName: unit.forms[0].name, names: [...unit.forms.map(form => form.name), ...unit.aliases], formCount: unit.forms.length, sharedForms: shared[i], lookupIds: [String(i)], url: `https://jarjarblink.github.io/JDB/u000.html?cc=ja&unit=${unit.id}` };
  });
  const aliasIndex = new Map(); for (const row of JSON.parse(aliases)) {
    check(Number.isSafeInteger(row.id) && row.id >= 0 && !aliasIndex.has(row.id) && Array.isArray(row.names) && row.names.every(value => typeof value === 'string' && value.trim()), 'Invalid enemy aliases'); aliasIndex.set(row.id, row.names);
  }
  const tut = []; lines(enemy).forEach((value, id) => {
    check(!/[\t\x00-\x1F\x7F]/.test(value), 'Invalid enemy name'); if (!value.trim()) return;
    const displayName = value.trim(); tut.push({ id: String(id), displayName, names: [displayName, ...new Set((aliasIndex.get(id) ?? []).filter(alias => alias !== displayName))], lookupIds: [String(id)], url: `https://jarjarblink.github.io/JDB/t000.html?cc=ja&unit=${id}` });
  }); check(tut.length > 0, 'Empty enemies');
  const rows = lines(types); check(rows.shift().trim().toLowerCase() === 'from,to,type' && rows.length <= 64, 'Invalid stage header');
  const ranges = rows.map(row => {
    const cells = row.split(',').map(value => value.trim()); check(cells.length === 3, 'Invalid stage range');
    const [from, to] = cells.slice(0, 2).map(integer), type = cells[2]; check(to >= from && to - from <= 999 && /^[A-Za-z][A-Za-z0-9_]*$/.test(type), 'Invalid stage type'); return { from, to, type };
  }); ranges.push({ from: 0, to: 999, type: 'N' }); ranges.sort((a, b) => a.from - b.from);
  check(new Set(ranges.map(range => range.type.toLowerCase())).size === ranges.length && ranges.every((range, i) => !i || range.from > ranges[i - 1].to), 'Overlapping stage types');
  const chapters = [['0', 3000], ['1', 3003], ['2', 3006], ['0Z', 20000], ['1Z', 21000], ['2Z', 22000]].map(([type, from]) => ({ type, from }));
  const reserved = [...chapters.flatMap(({ from }) => [from, from + 1, from + 2]), 23000, 38000];
  check(!ranges.some(range => reserved.some(id => id >= range.from && id <= range.to)), 'Reserved range overlap');
  function route(raw) {
    const range = ranges.find(range => raw >= range.from && raw <= range.to);
    if (range) return { type: range.type, map: raw - range.from, displayType: range.type };
    const special = chapters.find(range => raw >= range.from && raw <= range.from + 2);
    if (special) return { type: special.type, map: raw - special.from, displayType: null };
    if (raw === 23000 || raw === 38000) { const type = raw === 23000 ? '2_Inv' : '2Z_Inv'; return { type, map: 0, displayType: type }; }
    throw new Error(`Unmapped stage ID ${raw}`);
  }
  function entry(raw, route, displayName, names, stage) {
    const tail = stage === undefined ? '' : `:stage:${stage}`;
    return { id: `${route.displayType ? route.displayType + pad(route.map) : raw}${stage === undefined ? '' : `-${pad(stage)}`}`, displayName, names,
      lookupIds: [`raw:${raw}${tail}`, ...(route.displayType ? [`type:${route.displayType.toLowerCase()}:${route.map}${tail}`] : [])],
      url: `https://jarjarblink.github.io/JDB/map.html?cc=ja&type=${route.type}&map=${route.map}${stage === undefined ? '' : `&stage=${stage}`}`,
      ...(stage === undefined ? { rawUrl: `https://jarjarblink.github.io/JDB/map.html?cc=ja&id=${raw}` } : {}) };
  }
  const sales = idNames(saleNames, true);
  const maps = [...idNames(mapNames)].sort((a, b) => a[0] - b[0]).map(([raw, value]) => {
    const alias = raw >= 1000 ? sales.get(raw) : undefined;
    return entry(raw, route(raw), (value.includes('(旧)') || value.includes('（旧）')) ? value : alias ?? value, [...new Set([value, ...(alias ? [alias] : [])])]);
  });
  const documents = await map([...ranges.map(range => ({ ...range, normal: true })), ...chapters.map(range => ({ ...range, normal: false }))], async range => {
    let text = range.normal ? await read(`${site}/res/StageName_R${range.type}_ja.csv`, true) : await read(`${legacy}/data/StageName${range.type}_ja.csv`);
    if (text === null) text = await read(`${site}/res/StageName_${range.type}_ja.csv`);
    const rows = stageRows(text); check(range.normal ? rows.length - 1 <= range.to - range.from : rows.length === 3, 'Stage row range mismatch');
    return rows.flatMap((row, map) => row.flatMap((value, stage) => value ? [entry(range.from + map, { type: range.type, map, displayType: range.normal ? range.type : null }, value, [value], stage)] : []));
  });
  const stages = documents.flat().sort((a, b) => { const parts = e => e.lookupIds[0].split(':'); return Number(parts(a)[1]) - Number(parts(b)[1]) || Number(parts(a)[3]) - Number(parts(b)[3]); });
  const keys = [...maps, ...stages].flatMap(row => row.lookupIds); check(new Set(keys).size === keys.length, 'Duplicate stage keys');
  const entries = { ut, tut, st: [...maps, ...stages] };
  const revision = crypto.createHash('sha256').update(JSON.stringify(entries)).digest('hex');
  const source = { discordCommit: '02e6e9bbaeaabd93625a80056c372dc4f1733419', repositories: Object.fromEntries(repos.map((repo, i) => [repo, commits[i]])) };
  // 最初の資料の取得開始から2分。取得終了時刻で古い資料の寿命を延長しない。
  const payload = '\ufeff' + JSON.stringify({ schemaVersion: 2, revision, source, entries, ...(live ? { validatedAtMs: startedAt } : {}) }) + '\n'; check(Buffer.byteLength(payload) <= 4 * 1024 * 1024, 'Snapshot too large');
  fs.mkdirSync(path.dirname(output), { recursive: true }); fs.writeFileSync(output + '.tmp', payload); fs.renameSync(output + '.tmp', output);
  if (live) {
    for (const file of fs.readdirSync(sourceDirectory)) if (!usedSources.has(file)) fs.rmSync(path.join(sourceDirectory, file));
  }
  console.log(JSON.stringify({ output, bytes: Buffer.byteLength(payload), counts: Object.fromEntries(Object.entries(entries).map(([key, rows]) => [key, rows.length])), revision, source }));
}
main().catch(error => { console.error(error.message); process.exitCode = 1; });
