# FMD5Hash 对齐偏移问题说明

## 概述

`cache-registry-reader` 已实现对 `DevelopmentAssetRegistry.bin` 的基本解析。此前 PackageData 段从 Entry 2 起出现系统性字节偏移；根因已定位为 UE `FArchive` 将 `bool` 按 legacy `UBOOL`（`uint32`，4 字节）序列化，而解析器误按 1 字节读取。


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

### FMD5Hash 序列化 (SecureHash.h:147 + Archive.cpp:482)
```cpp
friend FArchive& operator<<(FArchive& Ar, FMD5Hash& Hash)
{
    Ar << Hash.bIsValid;                // bool，经 FArchive::SerializeBool 写为 uint32/4 bytes
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
    Ar << CookedHash;    // FMD5Hash = 4 + (0 or 16) bytes
//songhua start
    Ar << ReCook;        // bool，经 FArchive::SerializeBool 写为 uint32/4 bytes
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
FName(8) + DiskSize(8) + Guid(16) + bValid(uint32) + [hash(16 if bValid≠0)] + ReCook(uint32)

bValid=0 → 40 bytes/entry
bValid=1 → 56 bytes/entry

```

## 历史观测（基于 1 字节 bool 误解析）


### Entry 0 — 误判成功 (按旧逻辑 50 bytes, bValid=1)


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

### Entry 1 — 误判成功 (按旧逻辑 34 bytes, bValid=0)


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

### 失败的 Entry


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

这些跳转是错位后在名表索引范围内“偶然命中”的结果；真实根因不是变长模式交替，而是每条 Entry 固定少读两个 32-bit bool 中各 3 字节，共少读 6 字节。


## 根因结论

### UE `bool` 不是 1 字节持久化

`FArchive::SerializeBool` 明确将 `bool` 按 legacy `UBOOL` 序列化为 `uint32`：

```cpp
// Archive.cpp:482-497
void FArchive::SerializeBool(bool& D)
{
    uint32 OldUBoolValue = D ? 1 : 0;
    this->Serialize(&OldUBoolValue, sizeof(OldUBoolValue));
    D = !!OldUBoolValue;
}
```

因此：

- `FMD5Hash::bIsValid` 实际为 4 字节；
- LetsGo 定制的 `ReCook` 是 `bool`，同样实际为 4 字节；
- PackageData 条目长度应为：
  - `bValid=0`：`8 + 8 + 16 + 4 + 4 = 40` 字节；
  - `bValid=1`：`8 + 8 + 16 + 4 + 16 + 4 = 56` 字节。

解析器原先按 `u8` 读取两个 bool，每条 Entry 少消费 6 字节，导致后续 Entry 系统性错位。


## 修复验证方法

1. **回归测试**
   ```bash
   cargo test -p cache-registry-reader dev_package_data_consumes_four_byte -- --nocapture
   ```

2. **真实文件验证**
   ```bash
   cargo run -p cache-registry-reader -- "C:\MyTmp\letsgo-apk\Android-dev-1.6.40146.1\IntermediateData\PackInfo_Android_1.6.40146.1_Development\Metadata\DevelopmentAssetRegistry.bin" "./tmp/dev_output.json" --limit 1000
   ```
   期望 PackageData 不再从 Entry 2 起错位。

3. **金标准对比（可选）**
   - 用 `AssetRegistryViewerBin` 导出 JSON；
   - 提取前 100 条 PackageData 的 `PackageGuid`；
   - 与 Rust 工具输出逐条对比。


## 相关文件

| 文件 | 路径 |
|------|------|
| Rust 解析器 | `cache_registry_reader/src/main.rs` |
| 开发指南 | `cache_registry_reader/dev-guide-dev-registry.md` |
| UE 源码 - FMD5Hash | `Engine/Source/Runtime/Core/Public/Misc/SecureHash.h:147` |
| UE 源码 - bool 序列化 | `Engine/Source/Runtime/Core/Private/Serialization/Archive.cpp:482-505` |
| UE 源码 - FAssetPackageData | `Engine/Source/Runtime/CoreUObject/Public/AssetRegistry/AssetData.h:667-675` |
| UE 源码 - 加载端 | `Engine/Source/Runtime/AssetRegistry/Private/AssetRegistryState.cpp:2573-2589` |
| UE 源码 - 保存端 | `Engine/Source/Runtime/AssetRegistry/Private/AssetRegistryState.cpp:2460-2463` |
