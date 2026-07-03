# cache-registry-reader 开发指南

## 概述

解析 UE4 的 **CachedAssetRegistry.bin**（Editor 后台扫描线程生成的资产缓存文件），导出为 JSON。

> **注意：** 这不是 `DevelopmentAssetRegistry.bin`（Cook 产物），两者的二进制格式完全不同。
> `DevelopmentAssetRegistry.bin` 已有 `unreal_asset_registry` crate 解析，本工具面向的是
> Editor Intermediate 目录下的 `CachedAssetRegistry.bin`。

---

## 二进制格式（完整字节布局）

所有整数为 little-endian。

```
┌──────────────────────────────────────────────────────────────┐
│ FNameTableArchive Header (16 bytes)                          │
├──────────────────────────────────────────────────────────────┤
│ 0x00  int32   Magic           = 0x9E2A83C1 (PACKAGE_FILE_TAG)
│ 0x04  int32   Version         = 15 (CacheSerializationVersion)
│ 0x08  int64   NameTableOffset  字节偏移，指向文件末尾的名表
├──────────────────────────────────────────────────────────────┤
│ AssetRegistryVersion                                         │
├──────────────────────────────────────────────────────────────┤
│ 0x10  Guid    (16 bytes)      = {717F9EE7, E9B0493A, 88B39132, 1B388107}
│        └─ 4×uint32: A, B, C, D (little-endian)
│ 0x20  int32   Version         = 7 (FAssetRegistryVersion::AddedDependencyFlags)
├──────────────────────────────────────────────────────────────┤
│ Body (从 0x24 开始，所有 FName 都是名表索引)                   │
├──────────────────────────────────────────────────────────────┤
│ 0x24  int32   NumPackages     包数量
│
│ for each package:
│   ┌─ FName PackageName
│   │    int32 NameIndex        名表索引 (0-based)
│   │    int32 Number           FName 实例号 (通常为 0)
│   │
│   ├─ FDiskCachedAssetData:
│   │
│   │   int64  Timestamp        FDateTime (100ns ticks since 0001-01-01)
│   │
│   │   FName  Extension        包扩展名 (".uasset" / ".umap")
│   │
│   │   int32  AssetDataCount   该包内的资产数
│   │
│   │   for each FAssetData (SerializeForCache):
│   │   ┌─ FName ObjectPath     如 /Game/Path/Asset.Asset
│   │   │  FName PackagePath    如 /Game/Path
│   │   │  FName AssetClass     如 Blueprint, Texture2D
│   │   │  FName PackageName    如 /Game/Path/Asset
│   │   │  FName AssetName      如 Asset
│   │   │
│   │   │  TMap<FName, FString> TagsAndValues
│   │   │    int32 Count
│   │   │    for each: FName Key → FString Value
│   │   │      FString = int32 Len (负数=wide), then Len bytes
│   │   │
│   │   │  TArray<int32> ChunkIDs
│   │   │    int32 Count → [int32 × Count]
│   │   │
│   │   └─ uint32 PackageFlags
│   │
│   └─ FPackageDependencyData (SerializeForCache):
│        FName     PackageName
│        TArray<FObjectImport> ImportMap
│          每个 FObjectImport:
│            FName       ClassPackage
│            FName       ClassName
│            FPackageIndex OuterIndex  (int32)
│            FName       ObjectName
│            FName       PackageName   (WITH_EDITORONLY_DATA, Editor 版本必有)
│        TArray<FName> SoftPackageReferenceList
│        TMap<FPackageIndex, TArray<FName>> SearchableNamesMap
│        FAssetPackageData::SerializeForCache
│        TBitArray<> ImportUsedInGame      (int32 NumBits + byte data)
│        TBitArray<> SoftPackageUsedInGame
│
├─ Name Table (@NameTableOffset) ──────────────────────────────┤
│   int32   NameCount
│   for each FNameEntrySerialized:
│     FString  Name           (通过 Ar << FString: int32 Len + 数据)
│                              Len>0 → ANSI (Len bytes)
│                              Len<0 → UCS-2 Wide (|Len|×2 bytes)
│     uint16   NonCasePreservingHash  (已弃用，仍序列化)
│     uint16   CasePreservingHash     (已弃用，仍序列化)
└──────────────────────────────────────────────────────────────┘
```

---

## 可使用的基础组件

来自 `unreal_asset_base` 和 `unreal_helpers`：

### RawReader
```rust
use unreal_asset_base::reader::{ArchiveReader, RawReader};
use unreal_asset_base::containers::{NameMap, Chain, SharedResource};

let cursor = std::io::Cursor::new(file_bytes);
let name_map = SharedResource::new(NameMap::new());
let mut reader = RawReader::new(
    Chain::new(cursor, None),
    ObjectVersion::VER_UE4_25,  // 或从文件推断
    ObjectVersionUE5::default(),
    false,                       // use_event_driven_loader
    name_map.clone(),
);
```

### 关键读取方法
```rust
reader.read_i32::<LE>()?        // int32
reader.read_i64::<LE>()?        // int64
reader.read_u32::<LE>()?        // uint32
reader.read_fname()?            // FName (index+number, 解析为名表字符串)
reader.read_fstring()?          // Option<String> (FString 格式)
reader.read_array(|r| ...)?     // TArray: 先读 i32 count, 再读元素
reader.seek(SeekFrom::Start(n))? // 跳转
```

### 名表构建
```rust
// 加载名表条目到 NameMap
let name = reader.read_fstring()?;  // 先读字符串
name_map.get_mut().add_name_reference(name_string, false);
// 跳过 2×uint16 hash
reader.read_u16::<LE>()?;
reader.read_u16::<LE>()?;
```

---

## 实现计划

### Phase 1: 文件头 + 名表加载
- [ ] 验证 Magic (0x9E2A83C1) 和 Version (15)
- [ ] 读取 NameTableOffset，seek 到名表位置
- [ ] 解析所有 FNameEntrySerialized，构建 NameMap
- [ ] Seek 回 body 起始位置 (0x10)
- [ ] 验证 AssetRegistryVersion Guid + Version

### Phase 2: 身体解析
- [ ] 读取 NumPackages
- [ ] 循环解析每个包:
  - [ ] PackageName (FName)
  - [ ] Timestamp (int64)
  - [ ] Extension (FName)
  - [ ] AssetDataCount (int32)
  - [ ] 循环解析 FAssetData (5 FNames + TMap + TArray + uint32)
- [ ] 输出 JSON

### Phase 3: 依赖数据（可先跳过）
- [ ] FPackageDependencyData 解析
  - [ ] FObjectImport 解析
  - [ ] TBitArray 解析
  - [ ] FAssetPackageData 解析
- [ ] 依赖关系加入 JSON 输出

### Phase 4: 优化 & CLI
- [ ] 流式写 JSON（避免内存峰值）
- [ ] `--limit` / `--filter-path` / `--skip-dependencies` 参数
- [ ] 进度统计（3GB 文件需要）

---

## 关键 UE 源码参考

所有路径基于 UE4 引擎根目录：

```
D:/UGit/LetsGoDevelop/ue4_tracking_rdcsp/Engine/
```

| 相对于上述根目录的路径 | 行号 | 内容 |
|---|---|---|
| `Source/Runtime/AssetRegistry/Private/NameTableArchive.h` | — | FNameTableArchiveReader 完整实现 |
| `Source/Runtime/AssetRegistry/Private/NameTableArchive.cpp` | — | 名表读写逻辑 |
| `Source/Runtime/AssetRegistry/Private/AssetDataGatherer.cpp` | 1003–1067 | `SerializeCache`（身体格式） |
| `Source/Runtime/AssetRegistry/Private/DiskCachedAssetData.h` | — | `FDiskCachedAssetData` 结构 |
| `Source/Runtime/AssetRegistry/Private/PackageDependencyData.h` | — | `FPackageDependencyData::SerializeForCache` |
| `Source/Runtime/CoreUObject/Public/AssetRegistry/AssetData.h` | 485–499 | `FAssetData::SerializeForCache` |
| `Source/Runtime/Core/Public/UObject/NameTypes.h` | 283–329 | `FNameEntrySerialized` 结构 |
| `Source/Runtime/Core/Private/UObject/UnrealNames.cpp` | 2480–2561 | `FNameEntrySerialized operator<<` |

---

## 验证方法

### 1. 编译检查
```bash
cd D:/DWorkGit/unrealmodding
cargo build -p cache-registry-reader
```

### 2. 运行测试
```bash
cargo run -p cache-registry-reader -- "D:/LetsGoEditor/Editor/LetsGo/Intermediate/CachedAssetRegistry.bin" "D:/DWorkGit/letsgo_pak_analysis/Tmp/CachedAssetRegistry.assets.json" --limit 1000
```

### 3. 交叉验证
用 UE Editor Python API 导出小样本，与本工具输出对比：
- Asset 数量一致
- PackageName、AssetClass 一致
- 前 100 条逐字段比对

---

## 依赖关系图

```
cache-registry-reader
├── unreal_asset_base  (RawReader, NameMap, FName, ArchiveReader)
│   ├── unreal_helpers (UnrealReadExt, read_fstring_len, Guid)
│   │   └── byteorder
│   └── unreal_asset_proc_macro (derive macros)
├── serde / serde_json  (JSON output)
└── byteorder
```
