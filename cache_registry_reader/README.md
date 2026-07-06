# cache-registry-reader

UE4 资产注册表二进制解析器，将 `CachedAssetRegistry.bin` 和 `DevelopmentAssetRegistry.bin` 导出为 JSON。

> 支持 LetsGo UE4.26 定制格式（`FAssetRegistryVersion::AddedReCookFlags = 8`）。

## 快速开始

```bash
cargo build --release -p cache-registry-reader
```

### 基本用法

```bash
# 解析 CachedAssetRegistry.bin（格式自动检测）
cargo run --release -p cache-registry-reader -- path/to/CachedAssetRegistry.bin

# 解析 DevelopmentAssetRegistry.bin
cargo run --release -p cache-registry-reader -- path/to/DevelopmentAssetRegistry.bin

# 指定输出路径
cargo run --release -p cache-registry-reader -- input.bin -o output.json
```

默认输出到 `./tmp/<输入文件名>.json`。

## 命令行参数

| 参数 | 说明 |
|------|------|
| `<file>` | 输入的 `.bin` 文件路径 |
| `-o`, `--output <PATH>` | JSON 输出路径（默认 `./tmp/<文件名>.json`） |
| `--limit <N>` | 只导出前 N 个包/资产 |
| `-NoHardRefs` | 排除硬引用 |
| `-NoSoftRefs` | 排除软引用 |
| `-IncludeMetadata` | 包含资产元数据（TagsAndValues） |
| `--decode-fib-data` | 解码 FiBData 元数据（隐含 `-IncludeMetadata`） |
| `--class <A,B,...>` | 按资产类名过滤（精确匹配，不区分大小写，类名间 OR） |
| `--path <GLOB>` | 按包名 glob 过滤（不区分大小写，支持 `*` `?`） |

### 过滤示例

```bash
# 只导出 Blueprint 和 Texture2D
--class Blueprint,Texture2D

# 只导出 /Game/Characters 下的资产
--path /Game/Characters/*

# 按资产名过滤
--path */BP_Sword*

# 组合过滤（AND）
--class SkeletalMesh --path /Game/Characters/*/BP_Sword*
```

## 输出格式

```json
{
  "Metadata": {
    "ExportTime": "2025.01.15-10.30.00",
    "EngineVersion": "4.26.2 (registry v8)",
    "Tool": "cache-registry-reader",
    "Version": "0.1.0",
    "IncludeHardReferences": true,
    "IncludeSoftReferences": true,
    "IncludeMetadata": false,
    "TotalAssets": 513160
  },
  "Assets": [
    {
      "ObjectPath": "/Game/Path/Asset.Asset",
      "PackageName": "/Game/Path/Asset",
      "AssetName": "Asset",
      "AssetClass": "Blueprint",
      "PackagePath": "/Game/Path",
      "PackageGuid": "{75203B7A-4C2363A3-AC223B82-A9CBFA0A}",
      "DirectDependencies": {
        "Hard": ["/Script/Engine"]
      },
      "DependencyCount": 1
    }
  ]
}
```

说明：`DirectDependencies.Hard` / `DirectDependencies.Soft` 仅在对应数组非空时输出；`TagsAndValues` 仅在启用 `-IncludeMetadata` 且存在元数据时输出，未输出不等同于 JSON `null`。

## 支持的格式

| 格式 | 来源 | 特点 |
|------|------|------|
| `CachedAssetRegistry.bin` | Editor Intermediate 目录 | 含 Magic `0x9E2A83C1`，按包分组 |
| `DevelopmentAssetRegistry.bin` | Cook 产物 Metadata 目录 | 含 GUID `0x717F9EE7`，扁平资产列表 + DependsNode 图 |

格式自动检测，无需手动指定。

## FiBData 解码

元数据中的 `FiBData` 是 UE Find-in-Blueprints 搜索索引，默认只输出摘要。使用 `--decode-fib-data` 可解码为结构化 JSON：

```bash
cargo run --release -p cache-registry-reader -- input.bin \
  --path /Engine/ArtTools/RenderToTexture/Macros/* \
  -IncludeMetadata --decode-fib-data
```

详见 [dev-guide-fib-data.md](./dev-guide-fib-data.md)。

## 技术文档

- [dev-guide.md](./dev-guide.md) — 二进制格式详解 + 架构说明
- [dev-guide-fib-data.md](./dev-guide-fib-data.md) — FiBData 编码/解码细节

## 依赖

```
cache-registry-reader
├── unreal_asset_base (RawReader, NameMap, FName)
├── unreal_helpers (Guid, FString helpers)
├── serde / serde_json
└── byteorder
```

## License

本项目随 [unrealmodding](../../README.md) 主仓库一起发布。
