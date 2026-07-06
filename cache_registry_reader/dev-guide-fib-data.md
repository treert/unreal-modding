# FiBData 解析说明

本文说明 `CachedAssetRegistry.bin` / `DevelopmentAssetRegistry.bin` 中 `TagsAndValues.FiBData` 的特殊性、源码依据，以及 `cache-registry-reader` 当前如何处理该字段。

## 背景

`FiBData` 是 Unreal Editor 的 **Find in Blueprints** 搜索索引数据。

它不是普通业务元数据，也不是直接可读的 JSON。UE 会把蓝图中可被编辑器全局搜索的信息缓存到 Asset Registry Tag 中，用于在不加载 `.uasset` 的情况下支持蓝图搜索。

常见内容包括：

- Blueprint properties
- Graphs / Functions / Macros
- Nodes / Pins
- NodeGuid / ClassName / SchemaName
- 注释、默认值、显示名等搜索相关信息

因此，`FiBData` 往往非常长。某些蓝图或引擎宏资源中可能达到几十万字符。

## 特殊性：它是“二进制数据字符串化”

`FiBData` 外层类型是 `FString`，但语义上不是普通文本。

UE 会先用 `FArchive` 把数据序列化成二进制，再通过 `BytesToString` 把每个 byte 转成一个非零 `TCHAR`：

```text
TCHAR = byte + 1
```

这样做的目的是避免二进制中的 `0x00` 变成字符串终止符。

读取时，UE 用 `StringToBytes` 反向还原：

```text
byte = TCHAR - 1
```

所以输出中看到的内容会像“乱码”，例如：

```text
GjoeJoCmvfqsjouNbobhfs
Qspqfsujft
Obnf
```

按 `TCHAR - 1` 还原后分别是：

```text
FindInBlueprintManager
Properties
Name
```

这说明它不是 FString 读取错位，而是 UE 原本就这么编码。

## UE 源码依据

本地 UE4 源码根目录：

```text
D:/UGit/LetsGoDevelop/ue4_tracking_rdcsp/Engine/
```

关键源码：

| 路径 | 参考行 | 说明 |
|---|---:|---|
| `Source/Runtime/Core/Public/Containers/UnrealString.h` | `2166-2203` | `BytesToString` / `StringToBytes` 实现，确认 `byte + 1` / `TCHAR - 1` 规则 |
| `Source/Editor/Kismet/Private/FindInBlueprintManager.cpp` | `297-329` | `FiBSerializationHelpers::Serialize`，把 `FArchive` 序列化结果转成 FString |
| `Source/Editor/Kismet/Private/FindInBlueprintManager.cpp` | `3336-3381` | `FFindInBlueprintSearchManager::ConvertJsonStringToObject`，说明 FiBData 的整体结构和解析方式 |
| `Source/Editor/Kismet/Public/FindInBlueprintManager.h` | `117-138` | `EFiBVersion` / `FSearchDataVersionInfo` 定义 |

## 数据结构

UE 源码注释中说明，`FiBData` 的整体结构是：

```text
| int32 Version | int32 Size | TMap<int32, FText> LookupTable | Json String |
```

字段说明：

| 字段 | 类型 | 是否经过 BytesToString 编码 | 说明 |
|---|---|---|---|
| `Version` | `int32` | 是 | FiB 数据格式版本 |
| `Size` | `int32` | 是 | LookupTable 的字节长度 |
| `LookupTable` | `TMap<int32, FText>` | 是 | JSON 中字符串 ID 到真实 `FText` 的映射 |
| `Json String` | JSON 文本 | 否 | 搜索树主体，里面大量字符串值是 LookupTable 的数字 key |

注意：前 3 段是通过 `BytesToString` 编码的“字节字符串”；最后的 JSON 主体已经是正常 JSON 文本。

## 解析流程

`cache-registry-reader` 的解析流程：

1. 读取 `TagsAndValues` 时仍按 `TMap<FName, FString>` 读取，保证游标位置正确。
2. 如果 tag key 不是 `FiBData`，保持普通字符串输出。
3. 如果 tag key 是 `FiBData`：
   - 默认不展开，输出摘要字符串。
   - 传入 `--decode-fib-data` 时进行结构化解码。

### 默认输出

默认情况下，即使使用 `-IncludeMetadata`，`FiBData` 也不会原样输出超长编码串，而是输出类似：

```json
"FiBData": "特殊字符串：FiBData 是 UE Find-in-Blueprints 编码数据；encoded_chars=507417, version=1, lookup_table_bytes=422312, json_chars=85097；使用 --decode-fib-data 展开。"
```

这样可以避免 JSON 文件被大量不可读字符撑爆。

### `--decode-fib-data` 输出

使用：

```powershell
cargo run --release -p cache-registry-reader -- D:/LetsGoEditor/Editor/LetsGo/Intermediate/CachedAssetRegistry.bin --path /Engine/ArtTools/RenderToTexture/Macros/RenderToTextureMacros -IncludeMetadata --decode-fib-data
```

`FiBData` 会输出为对象：

```json
"FiBData": {
  "Type": "DecodedFiBData",
  "EncodedLength": 507417,
  "Version": 1,
  "LookupTableByteSize": 422312,
  "LookupTableEntryCount": 8633,
  "LookupTable": {
    "0": "FindInBlueprintManager",
    "1": "Properties"
  },
  "Json": {
    "0": [],
    "1": []
  }
}
```

其中：

- `LookupTable` 是从 `TMap<int32, FText>` 解出来的数字 ID 到文本的映射。
- `Json` 是 FiB 搜索树主体，里面的字符串数字需要结合 `LookupTable` 理解。
- 当前实现主要支持 UE Find-in-Blueprints 常见的 `FTextHistoryType::Base` 和 `None`。

## 实现注意事项

### 1. 不能用普通 UTF-8 语义理解 FiBData

`FiBData` 中大量控制字符是正常现象。它们来自 `byte + 1` 后的编码结果，不代表读串失败。

### 2. `read_fstring_unlimited` 是必要的

项目里的通用 `read_fstring` 对长度有保护限制，而 `FiBData` 可能超过该限制。

因此 `TagsAndValues` 中的 FString 读取使用 `read_fstring_unlimited`，否则遇到大型蓝图索引时会失败。

### 3. JSON 主体不是直接人类可读

虽然 `Json String` 本身是 JSON，但里面大量字段和值都是 lookup ID，例如：

```json
{"0":"1"}
```

需要用 `LookupTable` 把 `"0"`、`"1"` 映射回真实文本。

当前工具先保留原始结构：

- `LookupTable` 单独输出。
- `Json` 单独输出。

后续如果需要更人类可读，可以再加一个“替换 lookup id”的二次展开模式。

### 4. 解码失败时不让整个导出失败

如果遇到暂不支持的 `FTextHistoryType` 或异常数据，工具会输出：

```json
"FiBData": {
  "Type": "FiBDataDecodeError",
  "EncodedLength": 12345,
  "Error": "..."
}
```

这样可以保留其它资产导出结果。

## 与 UE 源码逻辑的对应关系

| UE 逻辑 | Rust 工具逻辑 |
|---|---|
| `BytesToString` | `fib_chars_to_bytes` 反向处理，每个 char 减 1 |
| `Deserialize<int32>` | `read_fib_i32` |
| `Deserialize<TMap<int32, FText>>` | `read_fib_lookup_table` |
| `FJsonSerializer::Deserialize` | `serde_json::from_str` |
| `ConvertJsonStringToObject` | `decode_fib_data` |

## 后续可选增强

- 支持更多 `FTextHistoryType`，例如格式化文本、StringTable 文本等。
- 增加 `--decode-fib-data-expanded`，把 JSON 中的 lookup id 替换成人类可读文本。
- 对超大的 `LookupTable` / `Json` 增加截断或单独落盘选项。
- 输出 FiBData 解析统计，例如 node 数、graph 数、pin 数。
