"""旧ログを可逆な配列JSONLへ変換し、容量でまとめる。外部通信は行わない。"""
import argparse
import gzip
import hashlib
import json
import re
import sqlite3
import tempfile
from pathlib import Path

FIELDS = ["createdAt", "id", "senderMid", "content", "contentType", "senderName", "metadata"]
CONTEXT = ["kind", "chatMid", "scopeMid", "chatType"]
DAMAGED = set()


def dump(value):
    return json.dumps(value, ensure_ascii=False, separators=(",", ":"), sort_keys=True)


def load(path):
    if path.stat().st_size > 32 * 1024 * 1024:
        raise ValueError("LegacyFileLimit")
    text = path.read_text(encoding="utf-8-sig")
    decoder, offset, values = json.JSONDecoder(), 0, []
    while offset < len(text):
        while offset < len(text) and (text[offset].isspace() or text[offset] == "\ufeff"):
            offset += 1
        if offset == len(text):
            break
        try:
            value, offset = decoder.raw_decode(text, offset)
        except json.JSONDecodeError:
            if not values:
                raise
            DAMAGED.add(path)
            break
        values.append(value)
    if len(values) == 1:
        return values[0]
    if not values:
        raise ValueError("EmptyLegacyFile")
    result = dict(values[0])
    for value in values[1:]:
        for key in CONTEXT:
            if result.get(key) != value.get(key):
                raise ValueError("ConcatenatedContextMismatch")
        for key in ("messages", "members", "events", "users"):
            if key in value:
                result.setdefault(key, []).extend(value[key])
    return result


def safe(value):
    if not re.fullmatch(r"[A-Za-z0-9_.-]{1,128}", str(value)):
        raise ValueError("InvalidLogScope")
    return str(value)


def implicit_context(stream):
    parts = stream.split("/")
    if parts[0] == "talk":
        scope = parts[1]
        chat = parts[-2] if parts[-2] != scope else scope
        return {"kind": "talk", "chatMid": chat, "scopeMid": scope,
                "chatType": {"u": "USER", "c": "GROUP", "r": "ROOM"}.get(chat[0], "GROUP")}
    square = parts[0] if parts[0].startswith("s") else parts[1]
    chat = parts[1] if len(parts) > 2 and parts[1].startswith("m") else square
    return {"kind": "square", "chatMid": chat, "scopeMid": square, "chatType": "SQUARE",
            "scope": "chat" if chat.startswith("m") else "square"}


def encode_message(record, context):
    extra = {k: v for k, v in record.items() if k not in FIELDS and (k not in context or v != context[k])}
    row = [record.get(k) for k in FIELDS]
    if isinstance(row[6], dict):
        metadata = dict(row[6])
        mask = 0
        for bit, key, value in [(1, "to", context["chatMid"]), (2, "toType", context["chatType"]),
                                (4, "squareChatMid", context["chatMid"]), (8, "squareMid", context["scopeMid"]),
                                (16, "eventCreatedTime", str(record.get("createdAt")))]:
            if key in metadata and metadata[key] == value:
                del metadata[key]
                mask |= bit
        row[6] = [metadata, mask]
    for key in FIELDS:
        if key in record and record[key] is None:
            extra[key] = None
    row.append(extra or None)
    while row and row[-1] is None:
        row.pop()
    return row


def decode_message(row, context):
    result = dict(context)
    result.update({k: v for k, v in zip(FIELDS, row) if v is not None})
    if isinstance(result.get("metadata"), list):
        metadata, mask = result["metadata"]
        metadata = dict(metadata)
        for bit, key, value in [(1, "to", context["chatMid"]), (2, "toType", context["chatType"]),
                                (4, "squareChatMid", context["chatMid"]), (8, "squareMid", context["scopeMid"]),
                                (16, "eventCreatedTime", str(result.get("createdAt")))]:
            if mask & bit:
                metadata[key] = value
        result["metadata"] = metadata
    if len(row) > 7 and row[7]:
        result.update(row[7])
    return result


class Migration:
    def __init__(self, root, output, target_bytes):
        self.root, self.output, self.target_bytes = root, output, target_bytes
        self.temp = tempfile.TemporaryDirectory(prefix="kbc-log-sort-")
        self.db = sqlite3.connect(Path(self.temp.name) / "sort.sqlite")
        self.db.executescript("PRAGMA journal_mode=OFF; PRAGMA cache_size=-8192; CREATE TABLE rows (stream TEXT, at INTEGER, key TEXT, payload TEXT, UNIQUE(stream,key));")
        self.streams = {}
        self.mapping = {}
        self.counts = {"messages": 0, "member-events": 0, "names": 0, "profiles": 0}
        self.duplicates = 0
        self.input_bytes = 0

    def add(self, stream, kind, context, at, key, row):
        descriptor = {"v": 1, "kind": kind, "context": context}
        if stream in self.streams and self.streams[stream] != descriptor:
            raise ValueError("ConflictingStreamContext")
        self.streams[stream] = descriptor
        payload = dump(row)
        old = self.db.execute("SELECT payload FROM rows WHERE stream=? AND key=?", (stream, key)).fetchone()
        if old:
            if old[0] != payload:
                # 同じIDでも旧記録の情報が異なる場合は捨てない。
                key += ":" + hashlib.sha256(payload.encode()).hexdigest()
            else:
                self.duplicates += 1
                return
        if self.db.execute("INSERT OR IGNORE INTO rows VALUES(?,?,?,?)", (stream, at, key, payload)).rowcount:
            self.counts[kind] += 1
        else:
            self.duplicates += 1

    def directory(self, kind, scope, chat=None):
        scope, chat = safe(scope), safe(chat) if chat else None
        if kind == "square":
            square = scope if scope.startswith("s") else self.mapping.get(chat or scope)
            base = square if square else "unmapped/" + scope
            return base + ("/" + chat if chat and chat.startswith("m") else "")
        return "talk/" + scope + ("/" + chat if chat and chat != scope else "")

    def read_sources(self):
        message_root = self.root / "logs/message-log"
        manifest = message_root / "manifest.json"
        if manifest.exists():
            for chat in load(manifest).get("chats", []):
                if chat.get("kind") == "square" and str(chat.get("scopeMid", "")).startswith("s"):
                    self.mapping[safe(chat["chatMid"])] = safe(chat["scopeMid"])
        sources = sorted(p for p in message_root.rglob("*.json") if p.name != "manifest.json")
        for path in sources:
            data = load(path)
            self.input_bytes += path.stat().st_size
            if not isinstance(data, dict) or "kind" not in data:
                raise ValueError("UnsupportedMessageFile")
            context = {key: data[key] for key in CONTEXT}
            base = self.directory(context["kind"], context["scopeMid"], context["chatMid"])
            for record in data.get("messages", []):
                canonical = {**context, **record}
                row = encode_message(canonical, context)
                if decode_message(row, context) != canonical:
                    raise ValueError("MessageRoundTripMismatch")
                self.add(base + "/messages", "messages", context, int(record["createdAt"]), str(record["id"]), row)
            for profile in data.get("members", []):
                self.add(base + "/profiles", "profiles", context, 0, str(profile["mid"]), profile)
        member_root = self.root / "logs/member-event-log"
        for path in sorted(p for p in member_root.rglob("*.json") if p.name != "manifest.json"):
            data = load(path)
            self.input_bytes += path.stat().st_size
            for event in data["events"]:
                scope = event.get("scope", "chat")
                base = self.directory("square", data["scopeMid"], data["chatMid"] if scope == "chat" else None)
                context = {"scope": scope, "scopeMid": data["scopeMid"]}
                # OC本体の通知元トークも保持し、イベントの所属と混同しない。
                row = [event["at"], event["type"], event["mid"], event.get("name"),
                       {k: v for k, v in event.items() if k not in ("at", "type", "mid", "name", "scope")} or None,
                       data["chatMid"] if scope == "square" else None]
                while row[-1] is None:
                    row.pop()
                self.add(base + "/member-events", "member-events", context, int(event["at"]), hashlib.sha256(dump(row).encode()).hexdigest(), row)
        names_path = self.root / "logs/member-name-history.json"
        if names_path.exists():
            self.input_bytes += names_path.stat().st_size
            for user in load(names_path)["users"]:
                base = self.directory(user["kind"], user["scopeMid"])
                for observation in user["names"]:
                    row = [observation["firstSeenAt"], user["mid"], None, observation["name"],
                           observation["lastSeenAt"], observation["count"],
                           {k: v for k, v in observation.items() if k not in ("firstSeenAt", "lastSeenAt", "count", "name")} or None]
                    while row[-1] is None:
                        row.pop()
                    self.add(base + "/names", "names", {"kind": user["kind"], "scopeMid": user["scopeMid"]},
                             0, hashlib.sha256(dump([user["mid"], observation]).encode()).hexdigest(), row)
        self.db.commit()

    def write(self, source_sha):
        if self.output.exists():
            raise ValueError("OutputMustBeNew")
        self.output.mkdir(parents=True)
        report = {"v": 1, "sourceSha": source_sha, "targetBytes": self.target_bytes,
                  "counts": self.counts, "duplicates": self.duplicates, "streams": 0, "files": 0,
                  "inputBytes": self.input_bytes, "outputBytes": 0, "roundTripVerified": True}
        for stream, descriptor in sorted(self.streams.items()):
            defaults = implicit_context(stream)
            descriptor = {**descriptor, "context": {k: v for k, v in descriptor["context"].items() if defaults.get(k) != v}}
            target = self.output / stream
            target.mkdir(parents=True)
            header = (dump(descriptor) + "\n").encode()
            rows, size, index, files = [], len(header), 1, []
            def flush():
                nonlocal rows, size, index
                if not rows:
                    return
                raw = header + b"".join(rows)
                packed = gzip.compress(raw, compresslevel=6, mtime=0)
                filename = f"{index:06d}.jsonl.gz"
                (target / filename).write_bytes(packed)
                # 保存したファイルを再度展開して全行を照合する。
                if gzip.decompress((target / filename).read_bytes()) != raw:
                    raise ValueError("WrittenLogMismatch")
                files.append({"name": filename, "count": len(rows), "rawBytes": len(raw), "bytes": len(packed),
                              "sha256": hashlib.sha256(packed).hexdigest()})
                report["files"] += 1
                report["outputBytes"] += len(packed)
                rows, size, index = [], len(header), index + 1
            for (payload,) in self.db.execute("SELECT payload FROM rows WHERE stream=? ORDER BY at,key", (stream,)):
                encoded = (payload + "\n").encode()
                if rows and size + len(encoded) > self.target_bytes:
                    flush()
                if len(encoded) > 8 * 1024 * 1024:
                    raise ValueError("LegacyRecordTooLarge")
                rows.append(encoded)
                size += len(encoded)
            flush()
            (target / "manifest.json").write_text(dump({**descriptor, "files": files, "active": files[-1]["name"] if files else None}), encoding="utf-8")
            report["streams"] += 1
        # backfill情報・member集約等の索引も一つに圧縮して残す。
        indexes = self.output / "legacy-indexes.json.gz"
        indexes.write_bytes(gzip.compress(dump({name: load(self.root / name) for name in
            ["logs/message-log/manifest.json", "logs/member-event-log/manifest.json"] if (self.root / name).exists()}).encode(), mtime=0))
        if DAMAGED:
            quarantine = self.output / "quarantine"
            quarantine.mkdir()
            for path in sorted(DAMAGED):
                raw = path.read_bytes()
                digest = hashlib.sha256(raw).hexdigest()
                (quarantine / (digest + ".json.gz")).write_bytes(gzip.compress(raw, mtime=0))
            report["damagedFilesPreserved"] = len(DAMAGED)
        report["outputBytes"] = sum(p.stat().st_size for p in self.output.rglob("*") if p.is_file())
        (self.output / "migration-report.json").write_text(dump(report), encoding="utf-8")
        return report


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--source-sha", required=True)
    parser.add_argument("--target-bytes", type=int, default=4 * 1024 * 1024)
    args = parser.parse_args()
    if not 1024 <= args.target_bytes <= 8 * 1024 * 1024:
        raise ValueError("InvalidChunkSize")
    migration = Migration(args.root.resolve(), args.output.resolve(), args.target_bytes)
    try:
        migration.read_sources()
        print(dump(migration.write(args.source_sha)))
    finally:
        migration.db.close()
        migration.temp.cleanup()


if __name__ == "__main__":
    main()
