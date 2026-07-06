# cache-registry-reader：DevelopmentAssetRegistry.bin 开发指南

## 概述

解析 UE4 的 **DevelopmentAssetRegistry.bin**（Cook 产物，位于 `Metadata/` 目录下），导出为 JSON。

> **与 CachedAssetRegistry.bin 的关系：** 两者内部使用相同的 `FNameTableArchive` 序列化格式。
> `DevelopmentAssetRegistry.bin` 是 `FAssetRegistryState::Serialize` 的直接产物，
> `CachedAssetRegistry.bin` 在外层多包了一层 `FNameTableArchive` 头 + 按包分组的 `FDiskCachedAssetData` 结构。

### 参考 UE 源码

```
D:/UGit/LetsGoDevelop/ue4_tracking_rdcsp/Engine/
```

| 路径 | 内容 |
|---|---|
| `Source/Programs/AssetRegistryViewerBin/Private/AssetRegistryViewerBin.cpp` | 官方 bin→json 转换工具（完整实现） |
| `Source/Runtime/AssetRegistry/Private/AssetRegistryState.cpp` | `FAssetRegistryState::Serialize` (行 2317-2602) |
| `Source/Runtime/AssetRegistry/Private/NameTableArchive.h` | `FNameTableArchiveReader` 完整实现 |
| `Source/Runtime/AssetRegistry/Private/NameTableArchive.cpp` | 名表读写逻辑 |
| `Source/Runtime/CoreUObject/Public/AssetRegistry/AssetData.h` | `FAssetData::SerializeForCache`, `FAssetPackageData`, `FAssetRegistryVersion` |

### FAssetRegistryVersion 版本号（LetsGo 定制版）

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
    AddedReCookFlags,           // 8  ← LetsGo 定制版本（测试文件使用此版本）
    VersionPlusOne,             // 9
    LatestVersion = VersionPlusOne - 1  // = 8
};
```

> **注意：** 这是 LetsGo 引擎的定制版本枚举。标准 UE4 的 LatestVersion = 7 (AddedDependencyFlags)。
> LetsGo 添加了 `AddedReCookFlags`(8)，在 `FAssetPackageData` 中增加了 `ReCook` 字段。

---

## 二进制格式（完整字节布局）

所有整数为 little-endian。

```
┌──────────────────────────────────────────────────────────────┐
│ FAssetRegistryVersionType Header (20 bytes)                  │
├──────────────────────────────────────────────────────────────┤
│ 0x00  Guid    (16 bytes)     = {717F9EE7, E9B0493A, 88B39132, 1B388107}
│        └─ 4×uint32: A, B, C, D (little-endian)
│ 0x10  int32   Version         = 8 (AddedReCookFlags，LetsGo 定制)
├──────────────────────────────────────────────────────────────┤
│ FNameTableArchive Body                                       │
├──────────────────────────────────────────────────────────────┤
│ 0x14  int64   NameTableOffset 指向文件末尾的名表（偏移量）
│ 0x1C  int32   AssetCount      资产数量
│
│ ── Asset Data（扁平列表，无按包分组） ──
│ for each asset (共 AssetCount 个):
│   ┌─ FAssetData::SerializeForCache:
│   │
│   │   FName ObjectPath        如 /Game/Path/Asset.Asset
│   │     int32 NameIndex       名表索引 (0-based)
│   │     int32 Number          FName 实例号 (通常为 0)
│   │
│   │   FName PackagePath       如 /Game/Path
│   │   FName AssetClass        如 Blueprint, Texture2D
│   │   FName PackageName       如 /Game/Path/Asset
│   │   FName AssetName         如 Asset
│   │
│   │   TMap<FName, FString> TagsAndValues
│   │     int32 Count
│   │     for each: FName Key → FString Value
│   │       FString = int32 Len (0=空, 负数=wide, 正数=ANSI), then |Len|-1 bytes + null terminator
│   │
│   │   TArray<int32> ChunkIDs
│   │     int32 Count → [int32 × Count]
│   │
│   └─ uint32 PackageFlags
│
│ ── Dependency Section（DependsNode 图） ──
│
│ 如果 Version >= AddedDependencyFlags (7):
│   int64   DependencySectionSize   依赖段大小（字节数，用于跳转）
│   int32   NumDependsNodes         依赖节点数
│
│   for each DependsNode:
│     ┌─ FAssetIdentifier:
│     │    uint8  FieldBits         哪些字段存在 (bit0=PackageName, bit1=PrimaryAssetType, bit2=ObjectName, bit3=ValueName)
│     │    FName  PackageName       (如果 FieldBits & 0x01)
│     │    FName  PrimaryAssetType  (如果 FieldBits & 0x02)
│     │    FName  ObjectName        (如果 FieldBits & 0x04)
│     │    FName  ValueName         (如果 FieldBits & 0x08)
│     │
│     │  ┌─ Package Dependencies (Hard + Soft):
│     │  │   int32  InDependenciesCount
│     │  │   int32  SortIndexes[InDependenciesCount]  依赖节点索引
│     │  │   int32  FlagWordCount = ceil(InDependenciesCount * 3 / 32)
│     │  │   uint32 FlagWords[FlagWordCount]           每 3 bits 编码一个 EDependencyProperty
│     │  │     HARD_BIT = 001 (Hard|Game|Build)
│     │  │     SOFT_BIT = 010 (Game|Build)
│     │  │
│     │  ┌─ Name Dependencies:
│     │  │   int32  NameDependenciesCount
│     │  └─ int32  NameDependenciesIndex[Count]
│     │
│     │  ┌─ Manage Dependencies (Hard + Soft):
│     │  │   int32  ManageDependenciesCount
│     │  │   int32  ManageSortIndexes[Count]
│     │  └─ uint32 ManageFlagWords[...]  每 1 bit
│     │
│     └─ Referencers:
│          int32  ReferencersCount
│          int32  ReferencerIndex[Count]
│
│ ── Package Data ──
│
│   int32   NumPackageData    包数据条目数
│   for each:
│     FName   PackageName
│
│     ┌─ FAssetPackageData::SerializeForCache:
│     │
│     │   int64   DiskSize        磁盘大小
│     │   Guid    PackageGuid     (16 bytes)
│     │
│     │   [如果 Version >= AddedCookedMD5Hash (6)]:
│     │     FMD5Hash CookedHash
│     │       uint8  bIsValid (0 或 1)
│     │       [如果 bIsValid]: uint8 HashBytes[16]
│     │
│     └─ [如果 Version >= AddedReCookFlags (8)]:
│           uint8  ReCook         (LetsGo 定制，0 或 1)
│
├─ Name Table (@NameTableOffset) ─────────────────────────────┤
│   int32   NameCount
│   for each FNameEntrySerialized:
│     FString  Name           (通过 Ar << FString: int32 Len + 数据)
│                              Len>0 → ANSI (Len-1 bytes + null)
│                              Len<0 → UCS-2 Wide (|Len|-1 ×2 bytes + null)
│     uint16   NonCasePreservingHash  (如果 ObjectVersion >= NAME_HASHES_SERIALIZED)
│     uint16   CasePreservingHash     (如果 ObjectVersion >= NAME_HASHES_SERIALIZED)
└──────────────────────────────────────────────────────────────┘
```

---

## 与 CachedAssetRegistry.bin 的对比

| 特性 | CachedAssetRegistry.bin | DevelopmentAssetRegistry.bin |
|------|--------------------------|------------------------------|
| **来源** | Editor Intermediate 目录 | Cook 产物 Metadata 目录 |
| **头部** | Magic(4) + HeaderVersion(4) + NameTableOffset(8) | Guid(16) + Version(4) + NameTableOffset(8) |
| **Body 起始** | 0x10 (GUID+Version 前有额外 8 字节验证) | 0x1C (AssetCount) |
| **资产组织** | 按包分组：PackageName → Timestamp → Extension → [AssetData...] → DepData | 扁平列表：[AssetData...] → DepNodes → PackageData |
| **依赖格式** | `FPackageDependencyData` (ImportMap-based) | `FDependsNode` 图 (索引引用) |
| **PackageData** | 嵌入在每个包的 `FPackageDependencyData` 中 | 独立数组，末尾单独序列化 |
| **名表格式** | 完全相同（FNameEntrySerialized） | 完全相同（FNameEntrySerialized） |
| **FName 格式** | 完全相同（int32 index + int32 number = 8 bytes） | 完全相同（int32 index + int32 number = 8 bytes） |

---

## 实现策略

### 核心思路

由于两种格式共享相同的 `FNameTableArchive` 子结构，可以复用名表加载、FName 读取、AssetData 解析等核心逻辑。主要差异在于：

1. **头部检测**：
   - `CachedAssetRegistry.bin`：前 4 字节 == `0x9E2A83C1` (LE)
   - `DevelopmentAssetRegistry.bin`：前 4 字节 == `0x717F9EE7` (LE, GUID[0])

2. **头部解析**：
   - `CachedAssetRegistry.bin`：Magic → HeaderVersion → NameTableOffset → GUID → Version → Body
   - `DevelopmentAssetRegistry.bin`：GUID → Version → NameTableOffset → Body

3. **身体解析**：
   - `CachedAssetRegistry.bin`：按包分组，每个包包含 FDiskCachedAssetData + FPackageDependencyData
   - `DevelopmentAssetRegistry.bin`：扁平资产列表 + DependsNode 图 + PackageData 数组

### 建议的代码结构

```
main.rs
├── 格式检测 (detect_format)
│   ├── Magic == 0x9E2A83C1 → Cached
│   └── GUID[0] == 0x717F9EE7 → Development
│
├── 共享工具函数 (已存在)
│   ├── read_fname_str        (复用)
│   ├── read_fstring_unlimited (复用)
│   ├── load_name_table       (复用)
│   ├── read_guid_str         (复用)
│   └── skip_bitarray         (复用)
│
├── CachedAssetRegistry.bin 解析 (已有)
│   ├── parse_cached_header
│   ├── parse_all_assets (package-centric)
│   └── parse_dependency_data
│
└── DevelopmentAssetRegistry.bin 解析 (新增)
    ├── parse_dev_header       (GUID + Version + NameTableOffset)
    ├── parse_dev_assets       (flat list, 复用 parse_asset_core)
    ├── parse_dev_depends_nodes (skip or extract package names)
    └── parse_dev_package_data  (extract PackageGuid)
```

### 依赖数据策略

`DevelopmentAssetRegistry.bin` 的依赖使用 `FDependsNode` 图格式，解析复杂度较高。建议分阶段实现：

- **Phase 1**：跳过依赖段（读取 `DependencySectionSize` 后直接 seek 到 PackageData 段），导出资产基础信息（不含依赖）
- **Phase 2**：解析 DependsNode 图，为每个资产匹配对应的依赖（通过 PackageName 在 DependsNode 中查找）

### FAssetRegistryVersion 版本兼容

测试文件使用版本 8（`AddedReCookFlags`，LetsGo 定制）。需要兼容的版本范围：
- **>= 4 (RemovedMD5Hash)**：可以解析（MD5Hash 被移除）
- **>= 6 (AddedCookedMD5Hash)**：PackageData 有 CookedHash 字段
- **>= 7 (AddedDependencyFlags)**：依赖使用 DependencySectionSize 包裹
- **>= 8 (AddedReCookFlags)**：PackageData 末尾有 ReCook 字段

---

## 实现计划

### Phase 1: 格式检测 + 头部解析
- [ ] 在 `main()` 中根据前 4 字节区分两种格式
- [ ] `DevelopmentAssetRegistry.bin`：验证 GUID、读取 Version
- [ ] 读取 NameTableOffset，seek 到名表位置加载名表
- [ ] Seek 回 body 起始位置 (0x1C)

### Phase 2: 资产数据解析
- [ ] 读取 AssetCount
- [ ] 扁平循环解析 FAssetData（复用 `parse_asset_core`，因为 FAssetData::SerializeForCache 格式相同）
- [ ] 验证前 N 条数据与 UE 官方工具输出一致

### Phase 3: 依赖数据
- [ ] 读取 DependencySectionSize，跳过依赖段
- [ ] 读取 NumPackageData
- [ ] 解析 FAssetPackageData（提取 PackageGuid 等）
- [ ] 将 PackageGuid 匹配到对应资产

### Phase 4: 完整依赖支持
- [ ] 解析 DependsNode 图
- [ ] 根据 PackageName 匹配硬/软依赖

---

## 验证方法

### 1. 编译检查
```bash
cd D:/DWorkGit/unrealmodding
cargo build -p cache-registry-reader
```

### 2. 运行测试（DevelopmentAssetRegistry.bin）
```bash
cargo run -p cache-registry-reader -- "C:\MyTmp\letsgo-apk\Android-dev-1.6.40146.1\IntermediateData\PackInfo_Android_1.6.40146.1_Development\Metadata\DevelopmentAssetRegistry.bin" "./tmp/dev_output.json" --limit 1000
```

### 3. 交叉验证
用 UE 引擎编译 `AssetRegistryViewerBin` 程序导出 JSON，与本工具输出对比：
- Asset 数量一致（目标：513,160 个资产）
- 前 100 条的 ObjectPath、PackageName、AssetClass 一致
- PackageGuid 格式一致

---

## 已知测试数据

| 文件 | 大小 | 版本 | 资产数 | 名表偏移 |
|------|------|------|--------|----------|
| `DevelopmentAssetRegistry.bin` (LetsGo Android 1.6.40146.1) | 340.86 MB | 8 (AddedReCookFlags) | ~513,160 | 0x0DE86AE9 (233,536,233) |

---

## 依赖关系图

```
cache-registry-reader
├── unreal_asset_base  (RawReader, NameMap, FName, ArchiveReader)
│   ├── unreal_helpers (Guid, fstring helpers)
│   │   └── byteorder
│   └── unreal_asset_proc_macro (derive macros)
├── serde / serde_json  (JSON output)
└── byteorder
```

> 注：`DevelopmentAssetRegistry.bin` 解析不需要 `unreal_asset_registry` crate。
> 因为两种格式共享 FNameTableArchive 结构，可以直接用 `RawReader` 手动解析。
