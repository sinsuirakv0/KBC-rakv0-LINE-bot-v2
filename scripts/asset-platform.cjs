const crypto = require('node:crypto');
const parse = text => JSON.parse(text.replace(/^\ufeff/, ''));
const safe = value => typeof value === 'string' && value.split('/').every(part => part && !['.', '..'].includes(part) && /^[A-Za-z0-9._@-]+$/.test(part));
function verify(text, proof) {
  const bytes = Buffer.from(text, 'utf8');
  if (!proof || bytes.length !== proof.size || crypto.createHash('sha256').update(bytes).digest('hex') !== proof.sha256) throw Error('Asset size/hash mismatch');
  return text;
}

// readは既存のHTTP枠・期限・ディスクETagを使い、本文をBOMも含めて保持する。
module.exports = async function openAssets(base, read) {
  const current = parse(await read(`${base}/metadata/jp/current.json`));
  const { datasetId, buildId, generatedPath } = current;
  const storageVersion = current.storageVersion ?? 1;
  if (![1, 2].includes(storageVersion) || (storageVersion === 2 && (!Number.isSafeInteger(current.versionCode) || current.versionCode <= 0))) throw Error('Unknown asset storage layout');
  const integrated = storageVersion === 2;
  const rawVersion = integrated ? String(current.versionCode) : datasetId;
  const generatedVersion = integrated ? rawVersion : buildId;
  if (current.schemaVersion !== 1 || current.status !== 'verified' || current.sourceRegion !== 'jp' || !safe(datasetId) || datasetId.includes('/')
      || !safe(buildId) || buildId.includes('/') || generatedPath !== `generated/jp/datasets/${generatedVersion}`) throw Error('Unverified asset dataset');
  const manifestText = await read(`${base}/${generatedPath}/manifest.json`);
  if (crypto.createHash('sha256').update(Buffer.from(manifestText)).digest('hex') !== current.manifestSha256) throw Error('Manifest hash mismatch');
  const manifest = parse(manifestText);
  const header = value => {
    if (!value?.header || ['datasetId', 'buildId', 'sourceRegion', 'versionCode', 'versionName'].some(key => value.header[key] !== current[key])) throw Error('Dataset header mismatch');
    return value;
  };
  header(manifest);
  const artifact = async path => header(parse(verify(await read(`${base}/${generatedPath}/${path}`, false, (path === 'indexes/assets.json' ? 16 : 10) * 1024 * 1024), manifest.artifacts[path])));
  const index = await artifact('indexes/assets.json');
  const files = Object.entries(index.files ?? {});
  if (!files.length || files.length > 131072 || files.some(([name, file]) => !safe(name) || !safe(file.path)
      || ![`jp/Local/${rawVersion}/`, 'jp/server/', 'overrides/jp/assets/', 'overrides/jp/music/'].some(prefix => file.path.startsWith(prefix)))) throw Error('Invalid asset source index');
  return { buildId, characters: await artifact('site/characters.json'), async text(path, optional = false) {
    if (!safe(path)) throw Error('Unsafe asset path');
    const file = index.files[path];
    if (!file) { if (optional) return null; throw Error(`Asset not selected: ${path}`); }
    return verify(await read(`${base}/${integrated ? `jp/Integrated-data/${path}` : file.path}`), file);
  } };
};
