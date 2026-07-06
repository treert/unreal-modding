# FMD5Hash 对齐偏移问题说明

## 概述

`cache-registry-reader` 已实现对 `DevelopmentAssetRegistry.bin` 的基本解析，但 PackageData 段的 FMD5Hash 可变长度序列化导致 Entry 2 起出现系统性字节偏移。需要定位偏移根因。

## 测试文件

```
C:\MyTmp\letsgo-apk\Android-dev-1.6.40146.1\IntermediateData\
  PackInfo_Android_1.6.40146.1_Development\Metadata\DevelopmentAssetRegistry.bin

大小: 357,417,537 bytes (340.86 MB)
版本: 8 (AddedReCookFlags, LetsGo 定制)
名表条目: 1,418,017
资产数: 513,160
PackageData 条目数: 512,037
名表偏移: 0xDE86AE9 (233,536,233)
```

## 已确认的格式（UE 引擎源码验证）

### FMD5Hash 序列化 (SecureHash.h:147)
```cpp
friend FArchive& operator<<(FArchive& Ar, FMD5Hash& Hash)
{
    Ar << Hash.bIsValid;                // 1 byte (bool)
    if (Hash.bIsValid)
        Ar.Serialize(Hash.Bytes, 16);   // 16 bytes (uint8[16])
    return Ar;
}
```

### FAssetPackageData::SerializeForCache (AssetData.h:667-675)
```cpp
void SerializeForCache(FArchive& Ar)
{
    Ar << DiskSize;      // int64 = 8 bytes
    Ar << PackageGuid;   // FGuid = 16 bytes
    Ar << CookedHash;    // FMD5Hash = 1 + (0 or 16) bytes
//songhua start
    Ar << ReCook;        // bool = 1 byte
//songhua end
}
```

### 加载端 (AssetRegistryState.cpp:2573-2589)
```cpp
if (Version < AddedCookedMD5Hash)       // v<6
    { DiskSize; PackageGuid; }           // 无 CookedHash、无 ReCook
else if (Version < AddedReCookFlags)    // v<8
    { DiskSize; PackageGuid; CookedHash; } // 无 ReCook
else                                    // v≥8
    { SerializeForCache(Ar); }           // 全部 4 个字段
```

### 保存端 (AssetRegistryState.cpp:2460-2463)
```cpp
for (TPair<FName, FAssetPackageData*>& Pair : CachedPackageData)
{
    Ar << Pair.Key;                     // FName = 8 bytes
    Pair.Value->SerializeForCache(Ar);  // 全部 4 个字段（无条件）
}
```

**结论：保存端和加载端格式完全一致，每条目格式为：**

```
FName(8) + DiskSize(8) + Guid(16) + bValid(1) + [hash(16 if bValid≠0)] + ReCook(1)

bValid=0 → 34 bytes/entry
bValid=1 → 50 bytes/entry
```

## 正常解析的 Entry

### Entry 0 — 成功 (50 bytes, bValid=1)

```
位置: 0xC33F871
Hex:
  00: 03 00 00 00 00 00 00 00  → FName: index=3, number=0  → "/Game/InitBank"
  08: 16 0A 00 00 00 00 00 00  → DiskSize = 2582
  10: 7A 3B 20 75 A3 63 23 4C 82 3B 22 AC 0A FA CB A9  → Guid: {75203B7A-4C2363A3-AC223B82-A9CBFA0A}
  20: 01                       → bValid = 1
  21: 00 00 00 22 53 80 81 57 1C 4B 03 BB DA 7A AE CA  → Hash (16 bytes)
  31: EA                       → ReCook = 234

✓ pkg=/Game/InitBank, guid={75203B7A-4C2363A3-AC223B82-A9CBFA0A}
```

### Entry 1 — 成功 (34 bytes, bValid=0)

```
位置: 0xC33F8A3 (= 0xC33F871 + 50) → 偏移正确 ✓
Hex:
  00: 91 CF 00 00 00 00 C7 97  → FName: index=53137, number=0x97C70000
  08: 15 00 00 00 00 00 00 00  → DiskSize = 21
  10: 00 00 00 00 00 00 C8 FF 12 21 6A B8 AD 35 00 00  → Guid: {00000000-FFC80000-B86A2112-000035AD}
  20: 00                       → bValid = 0 (无 hash 体)
  21: 00                       → ReCook = 0

✓ pkg=/Game/Feature/ChaseCommon/.../T_MCG_Detector_Purple_01_ARM
```

## 失败的 Entry

### Entry 2 — 失败 (bValid=114 → 异常)

```
位置: 0xC33F8C5 (= 0xC33F8A3 + 34) → 偏移正确 ✓
Hex:
  00: 00 00 00 00 00 00 00 00  → FName: index=0, number=0  → "/Game/InitBank.InitBank"
  08: 00 00 00 00 C8 97 15 00  → DiskSize = 0x001597C8_00000000 (≈5.5 PiB! 明显异常)
  10: 00 00 00 00 00 00 00 00 00 00 00 00 96 D3 7D D0  → Guid: {00000000-00000000-00000000-D07DD396}
  20: 72                       → bValid = 114 ← 应为 0 或 1!
  73 04 BA 00 00 00 00 00 00 00 00 00 00 00 00 00  (剩余字节)

✗ FName(OutOfRange(-1562247168, 1418017)) — 下一个 Entry 的 FName index 越界
```

### Entry 3 — 连锁失败

```
位置: 0xC33F8F7 (= 0xC33F8C5 + 50, bValid=114 被当作 1 处理)
Hex:
  00: 00 00 E2 A2 15 00 00 00  → FName: index=0xA2E20000 = -1562247168 ← 越界!
```

## 容错测试结果

当使用容错模式（跳过错位字节后重试解析）时：

```
PackageData: ~510,000 ok, ~257,000 errors
```

错误呈现交替模式：skip `2, 5, 2, 5, 3, 1, 3, 1, 5, 2, 2, 5, ...` 字节后能找到下一个有效 FName。

**交替 2/5 跳转模式暗示：** 相邻两条 Entry 的累计偏移为 7 字节（2+5）。如果一条 Entry 预期 50 字节但实际为 53 字节（多 3 字节），另一条 34 字节但实际为 38 字节（多 4 字节），则 3+4=7 正好匹配。

## 可能的原因假设

### 假设 1: ReCook 的 `bool` 在不同 Entry 中被序列化为不同大小

- 某些 Entry 的 ReCook 为 1 字节 (uint8)
- 某些 Entry 的 ReCook 为 4 字节 (int32，FArchive 在某些上下文下的行为)
- 这与交替偏移模式吻合

### 假设 2: FMD5Hash 的 bValid 在部分条目中为 4 字节

- `bValid` 本应是 uint8，但 FArchive 可能在某些情况下将 bool 序列化为 int32
- 如果 bValid=4 字节，条目大小从 34/50 变为 37/53

### 假设 3: 依赖段 (DependencySection) 跳过有 1 字节偏差

- `DependencySectionSize` 计算公式可能需要调整
- 但前两条 Entry 位置精确匹配，排除了明显的偏移

### 假设 4: 文件由不同版本的 UE 生成

- 源码显示 version=8 时走 `SerializeForCache(Ar)` 完整路径
- 但编译产物可能因宏定义不同而产生不同的序列化结果

## 建议验证方法

1. **编译 AssetRegistryViewerBin**（已有源码）
   ```
   D:/UGit/LetsGoDevelop/ue4_tracking_rdcsp/Engine/Source/Programs/
   AssetRegistryViewerBin/Private/AssetRegistryViewerBin.cpp
   ```
   用此程序导出 JSON 作为"金标准"参照

2. **对比输出**
   - 提取前 100 条 PackageData 的 PackageGuid
   - 与 Rust 工具的解析结果逐条比对
   - 定位第一条不一致的条目

3. **在 UE 编辑器内打印 PackageData 序列化字节**
   - 在 `AssetRegistryState::SerializeSaving` 中对每条 `FAssetPackageData` 打印 hex dump
   - 验证 ReCook 的实际序列化大小

## 相关文件

| 文件 | 路径 |
|------|------|
| Rust 解析器 | `cache_registry_reader/src/main.rs` |
| 开发指南 | `cache_registry_reader/dev-guide-dev-registry.md` |
| UE 源码 - FMD5Hash | `Engine/Source/Runtime/Core/Public/Misc/SecureHash.h:147` |
| UE 源码 - FAssetPackageData | `Engine/Source/Runtime/CoreUObject/Public/AssetRegistry/AssetData.h:667-675` |
| UE 源码 - 加载端 | `Engine/Source/Runtime/AssetRegistry/Private/AssetRegistryState.cpp:2573-2589` |
| UE 源码 - 保存端 | `Engine/Source/Runtime/AssetRegistry/Private/AssetRegistryState.cpp:2460-2463` |
