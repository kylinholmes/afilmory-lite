#!/usr/bin/env python3
"""
对比 afilmory-lite 产出的 manifest 里的 EXIF 与 exiftool 原始输出。

目的：验证「结构/语义一致」——我们保留的每个 EXIF 键，其值与 exiftool 逐字一致，
仅做了：① 白名单筛选（只保留 PICK_KEYS）② 3 处归一化（日期→ISO、GPSAltitudeRef→0/1、
ImageWidth/ImageHeight 取自 ExifImageWidth/ExifImageHeight）。

用法：
    python3 scripts/compare-exif.py <manifest.json> <images_base_dir> [exiftool_path]

例：
    python3 scripts/compare-exif.py /tmp/af2/work/manifest.json /home/ubuntu/afilmory-lite/images
"""
import json
import os
import subprocess
import sys

# 必须与 Rust 端 exif/exiftool.rs 的调用参数完全一致
EXIFTOOL_ARGS = ["-json", "-api", "largefilesupport=1"]

# 我们 Rust 端会对这些键做变换/派生，逐键比对时单独处理
TRANSFORMED = {"DateTimeOriginal", "DateTimeDigitized", "GPSAltitudeRef"}
DERIVED = {"ImageWidth": "ExifImageWidth", "ImageHeight": "ExifImageHeight"}
# exiftool 输出里我们本就不会保留的元信息键（不计入"被丢弃"统计）
META_KEYS = {"SourceFile", "ExifToolVersion", "Warning", "Error", "Directory",
             "FileName", "FilePermissions", "FileModifyDate", "FileAccessDate",
             "FileInodeChangeDate", "FileTypeExtension"}


def raw_exif(exiftool, path):
    out = subprocess.run([exiftool, *EXIFTOOL_ARGS, path], capture_output=True, text=True)
    if out.returncode != 0:
        return None, out.stderr.strip()
    arr = json.loads(out.stdout)
    return (arr[0] if arr else {}), None


def main():
    if len(sys.argv) < 3:
        print(__doc__)
        sys.exit(2)
    manifest_path, base = sys.argv[1], sys.argv[2]
    exiftool = sys.argv[3] if len(sys.argv) > 3 else "exiftool"

    manifest = json.load(open(manifest_path))
    total_keys = 0
    photos_checked = 0
    mismatches = []
    dropped_summary = []

    for it in manifest["data"]:
        ours = it.get("exif")
        if ours is None:
            print(f"[skip] {it['id']}: our exif is null（该图无 EXIF 或未抽取）")
            continue
        photo_path = os.path.join(base, it["s3Key"])
        raw, err = raw_exif(exiftool, photo_path)
        if raw is None:
            print(f"[err ] {it['id']}: exiftool 失败: {err}")
            continue
        photos_checked += 1

        # 1) 非变换键：逐字比对
        for k, v in ours.items():
            if k in TRANSFORMED or k in DERIVED:
                continue
            total_keys += 1
            if k not in raw:
                mismatches.append((it["id"], k, "MISSING_IN_EXIFTOOL", v, None))
            elif raw[k] != v:
                mismatches.append((it["id"], k, "VALUE_DIFF", v, raw[k]))

        # 2) 派生键：ImageWidth/Height 应等于 exiftool 的 ExifImageWidth/Height
        for k, rawk in DERIVED.items():
            if k in ours and rawk in raw and ours[k] != raw[rawk]:
                mismatches.append((it["id"], k, f"DERIVED_DIFF(vs {rawk})", ours[k], raw[rawk]))

        # 3) 统计被白名单丢弃的键（仅供信息）
        dropped = [k for k in raw if k not in ours and k not in META_KEYS]
        kept = len([k for k in ours if k in raw or k in DERIVED])
        dropped_summary.append((it["id"], kept, len(dropped)))

    print("\n=== 比对结果 ===")
    print(f"有 EXIF 的照片数: {photos_checked}")
    print(f"逐字核对的(非变换)键总数: {total_keys}")
    print(f"不一致数: {len(mismatches)}")
    for mid, k, kind, ours_v, raw_v in mismatches:
        print(f"  [{kind}] {mid} :: {k}: ours={ours_v!r} exiftool={raw_v!r}")

    print("\n=== 白名单筛选情况（每张：保留 / 丢弃）===")
    for mid, kept, dropped in dropped_summary:
        print(f"  {mid}: 保留 {kept} 键，丢弃 {dropped} 键")

    if not mismatches:
        print("\n✅ 所有保留的(非变换)EXIF 键值与 exiftool 完全一致")
        sys.exit(0)
    else:
        print(f"\n❌ 发现 {len(mismatches)} 处不一致（见上）")
        sys.exit(1)


if __name__ == "__main__":
    main()
