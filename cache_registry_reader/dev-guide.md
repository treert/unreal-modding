# cache-registry-reader：二进制格式与架构参考

## 概述

解析 UE4 的资产注册表二进制文件，支持两种格式：

| 格式 | 来源 | 头部 Magic/ID |
|------|------|--------------|
| `CachedAssetRegistry.bin` | Editor Intermediate 目录 | `0x9E2A83C1` |
| `DevelopmentAssetRegistry.bin` | Cook 产物 Metadata 目录 | `0x717F9EE7` (GUID[0]) |

两者共享相同的 `FNameTableArchive` 序列化基础设施（NameMap、FName 格式完全相同）。
`DevelopmentAssetRegistry.bin` 是 `FAssetRegistryState::Serialize` 的直接产物；
`CachedAssetRegistry.bin` 在外层多包了 `FNameTableArchive` 头 + 按包分组的 `FDiskCachedAssetData`。

> **源码根目录：** `D:/UGit/LetsGoDevelop/ue4_tracking_rdcsp/Engine/`

---

## FAssetRegistryVersion（LetsGo 定制）

```cpp
enum Type {
    PreVersioning = 0,
    HardSoftDependencies,       // 1
    AddAssetRegistryState,      // 2
    ChangedAssetData,           // 3
    RemovedMD5Hash,             // 4
    AddedHardManage,            // 5
    AddedCookedMD5Hash,         // 6
    AddedDependencyFlags,       // 7
    AddedReCookFlags,           // 8  ← LetsGo 定制
    VersionPlusOne,             // 9
    LatestVersion = 8
};
```

> 标准 UE4 的 `LatestVersion = 7`。LetsGo 添加了 `AddedReCookFlags(8)`，在 `FAssetPackageData` 末尾增加 `ReCook` 字段。

---

## 格式一：CachedAssetRegistry.bin

```
┌──────────────────────────────────────────────────────────────┐
│ FNameTableArchive Header (16 bytes)                          │
├──────────────────────────────────────────────────────────────┤
│ 0x00  int32   Magic           = 0x9E2A83C1                  │
│ 0x04  int32   Version         = 15                          │
│ 0x08  int64   NameTableOffset                                │
├──────────────────────────────────────────────────────────────┤
│ AssetRegistryVersion                                         │
├──────────────────────────────────────────────────────────────┤
│ 0x10  Guid    (16 bytes)                                     │
│ 0x20  int32   Version         = 7                           │
├──────────────────────────────────────────────────────────────┤
│ Body (从 0x24 开始)                                          │
├──────────────────────────────────────────────────────────────┤
│ 0x24  int32   NumPackages                                    │
│                                                              │
│ for each package:                                            │
│   FName     PackageName                                      │
│   int64     Timestamp (100ns ticks)                          │
│   FName     Extension (".uasset" / ".umap")                  │
│   int32     AssetDataCount                                   │
│                                                              │
│   for each FAssetData (SerializeForCache):                   │
│     FName   ObjectPath                                       │
│     FName   PackagePath                                      │
│     FName   AssetClass                                       │
│     FName   PackageName                                      │
│     FName   AssetName                                        │
│     TMap<FName, FString> TagsAndValues                       │
│     TArray<int32> ChunkIDs                                    │
│     uint32  PackageFlags                                     │
│                                                              │
│   FPackageDependencyData (SerializeForCache):                │
│     FName   PackageName                                      │
│     TArray<FObjectImport> ImportMap                          │
│     TArray<FName> SoftPackageReferenceList                   │
│     TMap<FPackageIndex, TArray<FName>> SearchableNamesMap    │
│     FAssetPackageData PackageData                            │
│     TBitArray<> ImportUsedInGame                             │
│     TBitArray<> SoftPackageUsedInGame                        │
│                                                              │
├─ Name Table (@NameTableOffset) ──────────────────────────────┤
│   int32   NameCount                                          │
│   for each: FString Name + uint16 Hash × 2                   │
└──────────────────────────────────────────────────────────────┘
```

### 硬依赖：通过 ImportMap + ImportUsedInGame 位图解析

- 遍历 ImportMap，对每个 FObjectImport 沿 `OuterIndex` 链向上解析 PackageName
- 跳过 `/Script/CoreUObject`、`/Script/Engine` 等通用脚本包
- 用 `ImportUsedInGame` 位图过滤仅游戏内用到的依赖
- 软依赖同理，用 `SoftPackageUsedInGame` 过滤 `SoftPackageReferenceList`

---

## 格式二：DevelopmentAssetRegistry.bin

```
┌──────────────────────────────────────────────────────────────┐
│ FAssetRegistryVersionType Header (20 bytes)                  │
├──────────────────────────────────────────────────────────────┤
│ 0x00  Guid    (16 bytes)                                     │
│ 0x10  int32   Version                                        │
├──────────────────────────────────────────────────────────────┤
│ 0x14  int64   NameTableOffset                                │
│ 0x1C  int32   AssetCount                                     │
│                                                              │
│ ── Asset Data（扁平列表）─────────────────────────────────── │
│ for each asset: FAssetData::SerializeForCache                │
│   (与 Cached 格式的单个 FAssetData 完全相同)                   │
│                                                              │
│ ── Dependency Section ────────────────────────────────────── │
│ [Version >= 7]:                                              │
│   int64   DependencySectionSize                              │
│   int32   NumDependsNodes                                    │
│   for each FDependsNode:                                     │
│     FAssetIdentifier (FieldBits + 可选 4×FName)               │
│     Package Dependencies (索引 + 5-bit flags per dep)        │
│       flag: Hard(0x1)|Game(0x2)|Build(0x4)...               │
│       → 只导出 Game 位为 1 的依赖                              │
│     Name Dependencies (索引数组)                              │
│     Manage Dependencies (索引 + 1-bit flags)                  │
│     Referencers (索引数组)                                    │
│                                                              │
│ ── Package Data ──────────────────────────────────────────── │
│   int32   NumPackageData                                     │
│   for each:                                                  │
│     FName(8) + DiskSize(8) + Guid(16)                        │
│     [v>=6] FMD5Hash: uint32 bIsValid + [16 bytes hash]       │
│     [v>=8] uint32 ReCook (LetsGo)                            │
│                                                              │
│   条目长度: v<6 → 32 bytes                                   │
│            v6-v7: bValid=0 → 36 bytes, bValid=1 → 52 bytes   │
│            v8+:   bValid=0 → 40 bytes, bValid=1 → 56 bytes   │
│                                                              │
├─ Name Table (@NameTableOffset) ──────────────────────────────┤
│   (与 Cached 格式完全相同)                                     │
└──────────────────────────────────────────────────────────────┘
```

### 依赖解析：DependsNode 图

- 每个 `FDependsNode` 有 `Identifier`（通常只有 `PackageName`）
- `PackageDependencies` 使用 packed 5-bit flags：
  - 仅导出带 `DEV_DEP_PROPERTY_GAME` 位的依赖
  - 按 `DEV_DEP_PROPERTY_HARD` 区分硬/软依赖

### PackageData：UE bool = legacy UBOOL = uint32

`FArchive::SerializeBool` 将 `bool` 按 legacy `UBOOL`（`uint32`，4 字节）序列化：

```cpp
// Archive.cpp:482-497
void FArchive::SerializeBool(bool& D) {
    uint32 OldUBoolValue = D ? 1 : 0;
    this->Serialize(&OldUBoolValue, sizeof(OldUBoolValue));
}
```

因此 `FMD5Hash::bIsValid` 和 LetsGo 定制的 `ReCook` 都是 4 字节，不是 1 字节。

---

## 两种格式对比

| 特性 | CachedAssetRegistry.bin | DevelopmentAssetRegistry.bin |
|------|------------------------|------------------------------|
| **来源** | Editor Intermediate | Cook Metadata |
| **头部** | Magic(4) + Version(4) + NameTableOffset(8) | GUID(16) + Version(4) + NameTableOffset(8) |
| **Body 起始** | 0x24 | 0x1C |
| **资产组织** | 按包分组 | 扁平列表 |
| **依赖格式** | ImportMap-based + BitArray 过滤 | DependsNode 图 + packed flags |
| **PackageData** | 嵌入每包的 DependencyData | 独立数组 |
| **名表** | 完全相同 | 完全相同 |

---

## 代码架构

```
main.rs
├── 格式检测 (前 4 字节)
│   ├── 0x9E2A83C1 → CachedAssetRegistry.bin
│   └── 0x717F9EE7 → DevelopmentAssetRegistry.bin
│
├── 共享组件
│   ├── load_name_table()      — 加载名表（两格式共用）
│   ├── parse_asset_core()     — 解析 FAssetData（两格式共用）
│   ├── read_fname_str()       — FName → String
│   ├── read_fstring_unlimited() — FString（无长度限制）
│   ├── read_guid_str()        — GUID → 格式化字符串
│   ├── read_bitarray()        — TBitArray 解析
│   └── FilterConfig           — --class / --path 过滤
│
├── Cached 解析路径
│   ├── parse_cached_registry()
│   ├── parse_all_assets()     — 按包循环
│   └── parse_dependency_data() — ImportMap + BitArray
│
├── Dev 解析路径
│   ├── parse_dev_registry()
│   ├── parse_dev_dependency_section() — DependsNode 图
│   └── read_dev_package_data_entry()  — PackageData
│
├── FiBData 处理
│   ├── summarize_fib_data()   — 默认摘要输出
│   ├── decode_fib_data()      — --decode-fib-data 结构化解码
│   ├── read_fib_lookup_table()
│   └── read_fib_lookup_text()
│
├── JSON 输出
│   ├── Output / OutputMetadata / AssetEntry / DepsContainer
│   └── metadata_tag_to_json_value()
│
└── Glob 匹配 (case-insensitive, 支持 * ?)
```

---

## 关键 UE 源码参考

| 路径（基于 Engine/） | 内容 |
|---|---|
| `Source/Runtime/AssetRegistry/Private/NameTableArchive.h` | FNameTableArchiveReader |
| `Source/Runtime/AssetRegistry/Private/AssetDataGatherer.cpp` | SerializeCache (Cached 格式) |
| `Source/Runtime/AssetRegistry/Private/AssetRegistryState.cpp` | Serialize (Dev 格式, 行 2317-2602) |
| `Source/Runtime/CoreUObject/Public/AssetRegistry/AssetData.h` | FAssetData, FAssetPackageData, FAssetRegistryVersion |
| `Source/Runtime/Core/Public/UObject/NameTypes.h` | FNameEntrySerialized |
| `Source/Runtime/Core/Private/Serialization/Archive.cpp` | SerializeBool (bool→uint32) |
| `Source/Runtime/Core/Public/Misc/SecureHash.h` | FMD5Hash 序列化 |
| `Source/Editor/Kismet/Private/FindInBlueprintManager.cpp` | FiBData 编码/解码 |
| `Source/Programs/AssetRegistryViewerBin/Private/AssetRegistryViewerBin.cpp` | 官方 bin→json 工具 |

---

## 验证方法

```bash
# 编译
cargo build -p cache-registry-reader

# 运行测试
cargo test -p cache-registry-reader

# 真实文件测试
cargo run --release -p cache-registry-reader -- input.bin --limit 1000
```
